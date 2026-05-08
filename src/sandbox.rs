use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::ErrorKind;

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("子目录违反沙箱规则: {0}")]
    Violation(String),
    #[error("io 错误: {0}")]
    Io(#[from] io::Error),
}

impl SandboxError {
    pub fn kind(&self) -> ErrorKind {
        match self {
            SandboxError::Violation(_) => ErrorKind::SandboxViolation,
            SandboxError::Io(_) => ErrorKind::InternalError,
        }
    }
}

/// Cross-platform default sandbox base directory.
/// macOS: ~/Library/Application Support/xld/downloads
/// Linux: $XDG_DATA_HOME/xld/downloads (fallback ~/.local/share/xld/downloads)
/// Windows: %LOCALAPPDATA%\xld\downloads
pub fn default_base_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("xld")
        .join("downloads")
}

/// Recursively create a directory if it does not exist.
pub fn ensure_dir(path: &Path) -> io::Result<()> {
    if path.exists() {
        Ok(())
    } else {
        std::fs::create_dir_all(path)
    }
}

/// Resolve a user-supplied subdirectory name within a sandbox base directory,
/// rejecting any input that could escape the sandbox.
///
/// Rules:
/// - reject `..` components
/// - reject absolute paths and Windows drive prefixes
/// - reject empty subdir parts
/// - resolved path must remain a descendant of `base` after canonicalization
pub fn resolve_subdir(base: &Path, subdir: Option<&str>) -> Result<PathBuf, SandboxError> {
    // 先 canonicalize base，保证所有返回路径都是绝对路径（base 不存在时退到 lexical）
    let canon_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());

    let subdir = match subdir {
        None => return Ok(canon_base),
        Some("") => return Ok(canon_base),
        Some(s) => s,
    };

    // Reject literal absolute paths early
    let candidate = Path::new(subdir);
    if candidate.is_absolute() {
        return Err(SandboxError::Violation(format!(
            "子目录不允许为绝对路径: {}",
            subdir
        )));
    }

    // Reject Windows drive prefixes / UNC explicitly even on non-Windows hosts
    // (defensive when the binary later runs on Windows)
    if subdir.contains(':') || subdir.starts_with('\\') {
        return Err(SandboxError::Violation(format!(
            "子目录包含非法路径符号: {}",
            subdir
        )));
    }

    // Walk components manually; any non-Normal component is illegal
    for component in candidate.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            _ => {
                return Err(SandboxError::Violation(format!(
                    "子目录包含非法组件: {}",
                    subdir
                )))
            }
        }
    }

    let resolved = canon_base.join(candidate);

    // Walk up from the deepest existing ancestor of `resolved`. As soon as we
    // find one that exists, canonicalize it and verify it stays under
    // `canon_base`. This catches symlinks placed inside the sandbox that
    // point outward.
    let mut probe = resolved.clone();
    loop {
        if probe.exists() {
            let probe_canon = probe.canonicalize().unwrap_or_else(|_| probe.clone());
            if !probe_canon.starts_with(&canon_base) {
                return Err(SandboxError::Violation(format!(
                    "解析后路径逃逸沙箱: {} (base: {})",
                    probe_canon.display(),
                    canon_base.display()
                )));
            }
            break;
        }
        if !probe.pop() {
            // Walked above the filesystem root — shouldn't happen when base exists.
            break;
        }
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_base_dir_is_under_data_dir() {
        let p = default_base_dir();
        let ds = p.display().to_string();
        assert!(ds.contains("xld"), "default_base_dir 应包含 xld: {}", ds);
        assert!(
            ds.contains("downloads"),
            "default_base_dir 应包含 downloads: {}",
            ds
        );
    }

    #[test]
    fn resolve_subdir_none_returns_base() {
        let dir = tempdir().unwrap();
        let canon = dir.path().canonicalize().unwrap();
        let r = resolve_subdir(dir.path(), None).unwrap();
        assert_eq!(r, canon);
    }

    #[test]
    fn resolve_subdir_canonicalizes_base_when_no_subdir() {
        let dir = tempdir().unwrap();
        let canon = dir.path().canonicalize().unwrap();
        let result = resolve_subdir(dir.path(), None).unwrap();
        assert_eq!(result, canon, "subdir=None 时也必须返回 canonical base");
    }

    #[test]
    fn resolve_subdir_canonicalizes_base_when_empty_subdir() {
        let dir = tempdir().unwrap();
        let canon = dir.path().canonicalize().unwrap();
        let result = resolve_subdir(dir.path(), Some("")).unwrap();
        assert_eq!(result, canon);
    }

    #[test]
    fn resolve_subdir_legal_subdir() {
        let dir = tempdir().unwrap();
        let canon = dir.path().canonicalize().unwrap();
        let r = resolve_subdir(dir.path(), Some("2026-05/topic-rust")).unwrap();
        // resolve_subdir canonicalizes base, so compare against canonical form
        assert!(
            r.starts_with(&canon),
            "resolved {} should start with canon base {}",
            r.display(),
            canon.display()
        );
        assert!(r.ends_with("2026-05/topic-rust"));
    }

    #[test]
    fn resolve_subdir_rejects_parent_traversal() {
        let dir = tempdir().unwrap();
        let r = resolve_subdir(dir.path(), Some("../etc"));
        assert!(matches!(r, Err(SandboxError::Violation(_))));
    }

    #[test]
    fn resolve_subdir_rejects_absolute_unix() {
        let dir = tempdir().unwrap();
        let r = resolve_subdir(dir.path(), Some("/tmp/leak"));
        assert!(matches!(r, Err(SandboxError::Violation(_))));
    }

    #[test]
    fn resolve_subdir_rejects_windows_drive_prefix() {
        let dir = tempdir().unwrap();
        let r = resolve_subdir(dir.path(), Some("C:\\Users"));
        assert!(matches!(r, Err(SandboxError::Violation(_))));
    }

    #[test]
    fn resolve_subdir_rejects_backslash_prefix() {
        let dir = tempdir().unwrap();
        let r = resolve_subdir(dir.path(), Some("\\foo"));
        assert!(matches!(r, Err(SandboxError::Violation(_))));
    }

    #[test]
    fn resolve_subdir_rejects_embedded_traversal() {
        let dir = tempdir().unwrap();
        let r = resolve_subdir(dir.path(), Some("safe/../../../escape"));
        assert!(matches!(r, Err(SandboxError::Violation(_))));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_subdir_detects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let base = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let link_path = base.path().join("escape_link");
        symlink(outside.path(), &link_path).unwrap();

        // Walking through the symlink to a child path: canonicalize will
        // resolve to the outside dir.
        std::fs::create_dir_all(outside.path().join("child")).unwrap();
        let r = resolve_subdir(base.path(), Some("escape_link/child"));
        assert!(
            matches!(r, Err(SandboxError::Violation(_))),
            "符号链接逃逸应被拒绝: {:?}",
            r
        );
    }

    #[test]
    fn ensure_dir_creates_recursively() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("a/b/c");
        ensure_dir(&target).unwrap();
        assert!(target.is_dir());
    }
}
