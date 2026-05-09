//! ETag cache 用于支撑 HTTP Range 续传与 SkippedExisting 快路径。
//!
//! 当 `download_media` 发现 `<final_path>.partial` 文件存在且 size > 0 时，必须先核对
//! cache 中记录的 ETag 与 server 当前 ETag，一致才能续传——否则远端内容可能已变更，
//! 拼接旧 partial + 新 chunk 会得到损坏的文件（`[old_partial][new_chunk]`）。
//!
//! 设计要点：
//!
//! - 单文件 JSON：`<cache_dir>/x_likes_downloader/etag-cache.json`
//! - 条目 key = `sha256(final_path)` 的 hex（避免文件名特殊字符 / 长度限制）
//! - 条目 value：`{etag, url, size, updated_at, finalized}`。`size` = server 资源
//!   完整大小（`Content-Length` 或 `Content-Range: bytes N-M/Total` 的 `Total`），
//!   **不**是 partial 文件大小（partial 大小直接 `metadata().len()` 读取）。
//!   `finalized` 区分 in-progress 元数据与已成功 rename 完成的下载——详见 EtagEntry
//!   字段注释
//! - 写入用 `fs2::FileExt::lock_exclusive` 独占锁，避免并发 binary 互踩；
//!   read-modify-write 全部在锁内（见 [`update_in_place`]）
//! - 读取容错：JSON 损坏视为 cache 缺失（空 cache 起），不 panic
//! - 无 GC：本版本不实施。用户可用 `find ~/Library/Caches/x_likes_downloader -size +1M -delete`
//!   或等价命令手工清理

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CACHE_FILENAME: &str = "etag-cache.json";
const CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtagEntry {
    pub etag: String,
    pub url: String,
    /// Server 资源完整大小（HTTP Content-Length 或 Content-Range Total）。
    /// **不**是当前 partial 文件大小。
    pub size: u64,
    pub updated_at: String,
    /// 标记此条目对应的下载是否已成功 rename 到 final_path。
    ///
    /// - `false`（默认）：headers 阶段写入的 in-progress 元数据。SkippedExisting
    ///   fast-path **不**能信任此条目——下载可能仍在进行 / 已中断 / 已失败。
    ///   resume 路径仍可用（etag/url/size 一致即可续传）
    /// - `true`：rename partial→final 成功后写入。fast-path skip 可信任此条目代表
    ///   一个完整的 v2.1+ 下载文件。
    ///
    /// 老 cache 文件无此字段时 `#[serde(default)]` 兜底为 false——保守地强制 HEAD
    /// verify，与升级路径行为一致。
    #[serde(default)]
    pub finalized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtagCache {
    pub version: u32,
    pub entries: HashMap<String, EtagEntry>,
}

impl Default for EtagCache {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            entries: HashMap::new(),
        }
    }
}

impl EtagCache {
    /// Cache 文件路径（按平台从 `dirs::cache_dir()` 派生）。
    /// 如果 base cache dir 不存在则创建（含子目录）。
    pub fn path() -> std::io::Result<PathBuf> {
        let base = dirs::cache_dir().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no platform cache dir (HOME / LOCALAPPDATA unset?)",
            )
        })?;
        let dir = base.join("x_likes_downloader");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(CACHE_FILENAME))
    }

    /// 读取 cache 文件；不存在或解析失败返回空 cache（不 panic）。
    /// 解析失败时往 stderr 写一行诊断（cache 重置不算 hard error）。
    pub fn load() -> Self {
        let path = match Self::path() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("etag_cache: 无法解析 cache 路径: {} (使用空 cache)", e);
                return Self::default();
            }
        };
        Self::load_from(&path)
    }

    /// 仅供测试 / 内部用：从指定路径读 cache。
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<EtagCache>(&text) {
                Ok(cache) if cache.version == CACHE_VERSION => cache,
                Ok(_) => {
                    eprintln!(
                        "etag_cache: {:?} version mismatch (expected {}); resetting",
                        path, CACHE_VERSION
                    );
                    Self::default()
                }
                Err(e) => {
                    eprintln!(
                        "etag_cache: {:?} JSON parse failed ({}); resetting",
                        path, e
                    );
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                eprintln!("etag_cache: {:?} read failed ({}); resetting", path, e);
                Self::default()
            }
        }
    }

    /// 写入 cache 文件，使用独占文件锁防并发损坏。
    /// 注意：此函数仅写入 `self` 内存中的状态——并发场景请用 [`update_in_place`]
    /// 把整个 read-modify-write 包在锁内。
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path()?;
        self.save_to(&path)
    }

    /// 仅供测试 / 内部用：写到指定路径。
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        FileExt::lock_exclusive(&file)?;
        let result = write_locked(&file, self);
        let _ = FileExt::unlock(&file);
        result
    }

    /// 计算 final_path 的 sha256 hex 作为 cache key。
    pub fn key_for(final_path: &Path) -> String {
        let mut hasher = Sha256::new();
        hasher.update(final_path.as_os_str().to_string_lossy().as_bytes());
        let bytes = hasher.finalize();
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes.iter() {
            use std::fmt::Write;
            let _ = write!(&mut s, "{:02x}", b);
        }
        s
    }

    pub fn get(&self, key: &str) -> Option<&EtagEntry> {
        self.entries.get(key)
    }

    pub fn set(&mut self, key: String, entry: EtagEntry) {
        self.entries.insert(key, entry);
    }

    pub fn remove(&mut self, key: &str) -> Option<EtagEntry> {
        self.entries.remove(key)
    }

    /// 构造一个 entry，updated_at 设为 now。默认 `finalized = false`（in-progress 元数据）；
    /// rename 成功后 caller 应当再调一次 `make_finalized_entry` 写入 finalized=true。
    pub fn make_entry(etag: impl Into<String>, url: impl Into<String>, size: u64) -> EtagEntry {
        EtagEntry {
            etag: etag.into(),
            url: url.into(),
            size,
            updated_at: Utc::now().to_rfc3339(),
            finalized: false,
        }
    }

    /// 构造一个 finalized entry（rename partial→final 成功后调用）。fast-path skip
    /// 仅信任 finalized=true 的条目。
    pub fn make_finalized_entry(
        etag: impl Into<String>,
        url: impl Into<String>,
        size: u64,
    ) -> EtagEntry {
        EtagEntry {
            etag: etag.into(),
            url: url.into(),
            size,
            updated_at: Utc::now().to_rfc3339(),
            finalized: true,
        }
    }
}

/// 偷懒读取 cache 文件中某 key 的当前内容（不加载整个 cache）。
/// 主要用于诊断 / 测试；生产路径应通过 `EtagCache::load_from` + `get` 拿。
pub fn peek_entry(path: &Path, key: &str) -> Option<EtagEntry> {
    let cache = EtagCache::load_from(path);
    cache.entries.get(key).cloned()
}

/// 把 read → modify → write 全包在文件锁里，多线程 / 多进程并发安全。
/// 用于实现 [`put_entry`] / [`remove_entry`]：调用方传入闭包，闭包在拿到当前 cache
/// 的可变引用时已持锁。
pub fn update_in_place<F>(path: &Path, mutate: F) -> std::io::Result<()>
where
    F: FnOnce(&mut EtagCache),
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    FileExt::lock_exclusive(&file)?;
    let result = (|| -> std::io::Result<()> {
        // Read current contents inside the lock
        file.seek(SeekFrom::Start(0))?;
        let mut buf = String::new();
        use std::io::Read as _;
        file.read_to_string(&mut buf)?;
        let mut cache = if buf.trim().is_empty() {
            EtagCache::default()
        } else {
            match serde_json::from_str::<EtagCache>(&buf) {
                Ok(c) if c.version == CACHE_VERSION => c,
                _ => EtagCache::default(),
            }
        };
        mutate(&mut cache);
        write_locked(&file, &cache)
    })();
    let _ = FileExt::unlock(&file);
    result
}

fn write_locked(file: &std::fs::File, cache: &EtagCache) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(cache).map_err(std::io::Error::other)?;
    file.set_len(0)?;
    let mut f = file;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    Ok(())
}

/// 在已加载的 cache 上插入一条 entry 并写回。read+write 全在文件锁内，并发安全。
pub fn put_entry(path: &Path, key: String, entry: EtagEntry) -> std::io::Result<()> {
    update_in_place(path, |cache| {
        cache.set(key, entry);
    })
}

/// 同上，但 remove。
pub fn remove_entry(path: &Path, key: &str) -> std::io::Result<()> {
    update_in_place(path, |cache| {
        cache.remove(key);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("etag-cache.json");
        let cache = EtagCache::load_from(&path);
        assert!(cache.entries.is_empty());
        assert_eq!(cache.version, CACHE_VERSION);
    }

    #[test]
    fn load_corrupted_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("etag-cache.json");
        std::fs::write(&path, b"{ not valid json").unwrap();
        let cache = EtagCache::load_from(&path);
        assert!(cache.entries.is_empty(), "corrupted cache must reset");
    }

    #[test]
    fn load_wrong_version_resets() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("etag-cache.json");
        let bad = serde_json::json!({ "version": 999, "entries": {} });
        std::fs::write(&path, serde_json::to_string(&bad).unwrap()).unwrap();
        let cache = EtagCache::load_from(&path);
        assert!(cache.entries.is_empty(), "wrong version must reset");
    }

    #[test]
    fn save_and_reload_roundtrips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("etag-cache.json");
        let mut cache = EtagCache::default();
        cache.set(
            "abc".into(),
            EtagCache::make_entry("\"v1\"", "https://x/1", 4096),
        );
        cache.save_to(&path).unwrap();
        let loaded = EtagCache::load_from(&path);
        assert_eq!(loaded.entries.len(), 1);
        let e = loaded.get("abc").unwrap();
        assert_eq!(e.etag, "\"v1\"");
        assert_eq!(e.size, 4096);
        assert_eq!(e.url, "https://x/1");
    }

    #[test]
    fn key_for_is_stable_and_hex() {
        let p = std::path::PathBuf::from("/sandbox/alice_123_video.mp4");
        let k1 = EtagCache::key_for(&p);
        let k2 = EtagCache::key_for(&p);
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // sha256 hex
        assert!(k1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn key_for_distinguishes_paths() {
        let p1 = std::path::PathBuf::from("/sandbox/a.mp4");
        let p2 = std::path::PathBuf::from("/sandbox/b.mp4");
        assert_ne!(EtagCache::key_for(&p1), EtagCache::key_for(&p2));
    }

    #[test]
    fn put_and_remove_entry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("etag-cache.json");
        let key = "k1".to_string();
        put_entry(
            &path,
            key.clone(),
            EtagCache::make_entry("\"e1\"", "https://x/1", 1024),
        )
        .unwrap();
        assert!(EtagCache::load_from(&path).get(&key).is_some());
        remove_entry(&path, &key).unwrap();
        assert!(EtagCache::load_from(&path).get(&key).is_none());
    }

    #[test]
    fn save_does_not_lose_other_entries_after_reload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("etag-cache.json");
        put_entry(
            &path,
            "k1".into(),
            EtagCache::make_entry("\"e1\"", "https://x/1", 1),
        )
        .unwrap();
        put_entry(
            &path,
            "k2".into(),
            EtagCache::make_entry("\"e2\"", "https://x/2", 2),
        )
        .unwrap();
        let cache = EtagCache::load_from(&path);
        assert_eq!(cache.entries.len(), 2);
    }

    /// 并发写入测试：多线程同时 put_entry 不应丢条目。
    #[test]
    fn concurrent_save_does_not_lose_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("etag-cache.json");
        let mut handles = Vec::new();
        for i in 0..8 {
            let p = path.clone();
            handles.push(std::thread::spawn(move || {
                let key = format!("k{}", i);
                put_entry(
                    &p,
                    key,
                    EtagCache::make_entry(format!("\"e{}\"", i), format!("https://x/{}", i), i),
                )
                .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // 锁串行化；最后写入者覆盖；但每次 put_entry 都先 load 再 save，
        // 所以最终应当含全部 8 条
        let cache = EtagCache::load_from(&path);
        assert_eq!(
            cache.entries.len(),
            8,
            "concurrent put_entry should not lose entries (entries: {:?})",
            cache.entries.keys().collect::<Vec<_>>()
        );
    }
}
