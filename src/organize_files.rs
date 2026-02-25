use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub struct FileOrganizer;

impl FileOrganizer {
    pub fn organize_files(a_dir: &str, b_dir: &str) -> Result<()> {
        let a_path = Path::new(a_dir);
        let b_path = Path::new(b_dir);

        if !a_path.exists() || !b_path.exists() {
            return Err(anyhow::anyhow!("源目录或目标目录不存在"));
        }

        // 构建目标文件夹映射：前缀 -> 文件夹全路径
        let mut b_folders = HashMap::new();
        for entry in fs::read_dir(b_path)? {
            let entry = entry?;
            let full_path = entry.path();
            if full_path.is_dir() {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                let prefix = if name.contains(' ') {
                    name.split(' ').next().unwrap_or(&name).to_string()
                } else {
                    name.to_string()
                };
                b_folders.insert(prefix, full_path);
            }
        }

        // 加载用户名别名映射
        let alias_file = a_path.join("username_aliases.txt");
        let aliases = Self::load_username_aliases(&alias_file.to_string_lossy());
        if !aliases.is_empty() {
            println!("已加载 {} 条用户名别名映射", aliases.len());
        }

        // 统计变量
        let mut moved_count = 0;
        let mut skipped_count = 0;
        let mut deleted_count = 0;
        let mut error_count = 0;

        // 遍历 a_dir 下所有文件进行移动
        for entry in fs::read_dir(a_path)? {
            let entry = entry?;
            let file_path = entry.path();

            if file_path.is_file() {
                let file_name = entry.file_name();
                let filename = file_name.to_string_lossy();

                // 跳过配置文件
                if filename == "username_aliases.txt" {
                    continue;
                }

                // 使用新的解析函数
                let parse_result = Self::parse_filename(&filename);

                match parse_result {
                    Ok((username, tweet_id)) => {
                        // 别名查询：若 username 在别名表中，替换为主名称
                        // 循环解析别名链，直到找到最终的主名称
                        let match_username = Self::resolve_alias_chain(&username, &aliases);
                        let is_alias = match_username != username;

                        // 查找对应的目标文件夹
                        // 优先精确匹配，然后再尝试前缀匹配
                        let mut target_folder = None;

                        // 首先尝试精确匹配
                        if let Some(folder_path) = b_folders.get(&match_username) {
                            target_folder = Some(folder_path);
                        } else if !is_alias {
                            // 只有在不是别名映射时才进行前缀匹配
                            // 如果是别名映射，必须精确匹配主名称文件夹
                            for (folder_prefix, folder_path) in &b_folders {
                                if match_username.starts_with(&format!("{}_", folder_prefix))
                                    || folder_prefix.starts_with(&format!("{}_", match_username))
                                {
                                    target_folder = Some(folder_path);
                                    break;
                                }
                            }
                        }

                        if let Some(target_folder_path) = target_folder {
                            let destination = target_folder_path.join(&*filename);

                            // 检查目标目录是否有同名文件
                            if destination.exists() {
                                match Self::handle_duplicate_file(&file_path, &destination) {
                                    Ok(_) => {
                                        deleted_count += 1;
                                    }
                                    Err(e) => {
                                        println!("  删除同名文件失败: {}", e);
                                        error_count += 1;
                                        continue;
                                    }
                                }
                            }

                            // 移动文件到目标目录
                            match fs::rename(&file_path, &destination) {
                                Ok(_) => {
                                    println!(
                                        "已将 {} 移动到 {:?} - 用户: {}, ID: {}",
                                        filename, target_folder_path, username, tweet_id
                                    );
                                    moved_count += 1;
                                }
                                Err(e) => {
                                    println!(
                                        "移动文件失败: {} -> {:?}, 错误: {}",
                                        filename, destination, e
                                    );
                                    error_count += 1;
                                }
                            }
                        } else {
                            if match_username != username {
                                println!(
                                    "未找到用户名为 {}（别名 {} 的主名称）的目标文件夹，跳过 {}",
                                    match_username, username, filename
                                );
                            } else {
                                println!(
                                    "未找到用户名为 {} 的目标文件夹，跳过 {}",
                                    username, filename
                                );
                            }
                            skipped_count += 1;
                        }
                    }
                    Err(error_msg) => {
                        println!("文件 {} 解析失败: {}", filename, error_msg);
                        skipped_count += 1;
                    }
                }
            }
        }

        // 打印处理统计
        println!("\n=== 文件整理统计 ===");
        println!("成功移动文件: {}", moved_count);
        println!("跳过文件: {}", skipped_count);
        println!("删除同名文件: {}", deleted_count);
        println!("处理错误: {}", error_count);

        Ok(())
    }

    fn resolve_alias_chain(username: &str, aliases: &HashMap<String, String>) -> String {
        let mut current = username.to_string();
        let mut visited = HashSet::new();

        while let Some(next) = aliases.get(&current) {
            if visited.contains(&current) {
                eprintln!(
                    "警告: 检测到别名循环，涉及用户名 '{}'. 使用原始用户名。",
                    username
                );
                return username.to_string();
            }
            visited.insert(current.clone());
            current = next.clone();
        }

        current
    }

    fn load_username_aliases(alias_file_path: &str) -> HashMap<String, String> {
        let path = Path::new(alias_file_path);
        if !path.exists() {
            return HashMap::new();
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("警告: 无法读取别名文件 {}: {}", alias_file_path, e);
                return HashMap::new();
            }
        };

        // 移除 UTF-8 BOM（如果存在）
        let content = content.strip_prefix('\u{FEFF}').unwrap_or(&content);

        let mut aliases = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let usernames: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if usernames.len() < 2 {
                continue;
            }
            let primary = usernames[0].to_string();
            if primary.is_empty() {
                continue;
            }
            for alias in &usernames[1..] {
                if !alias.is_empty() {
                    aliases.insert(alias.to_string(), primary.clone());
                }
            }
        }

        aliases
    }

    /// Tweet ID minimum digits (Twitter Snowflake ID: 18-19 digits, username max: 15 chars)
    const TWEET_ID_MIN_DIGITS: usize = 16;
    /// Fallback minimum for historical short tweet IDs (pre-2010 Twitter IDs)
    const TWEET_ID_MIN_DIGITS_FALLBACK: usize = 10;

    fn parse_filename(filename: &str) -> Result<(String, String), String> {
        let (filename_no_ext, _) = filename.rsplit_once('.').unwrap_or((filename, ""));
        let tokens: Vec<&str> = filename_no_ext.split('_').collect();

        if tokens.len() < 3 {
            return Err("文件名格式错误：至少需要3个部分 (a_b_c)".to_string());
        }

        // Find the first token with >= 16 pure ASCII digits as Tweet ID
        let tweet_id_index = tokens
            .iter()
            .position(|token| {
                token.len() >= Self::TWEET_ID_MIN_DIGITS
                    && token.chars().all(|c| c.is_ascii_digit())
            })
            // Fallback: accept >= 10 digits for historical short tweet IDs
            .or_else(|| {
                tokens.iter().position(|token| {
                    token.len() >= Self::TWEET_ID_MIN_DIGITS_FALLBACK
                        && token.chars().all(|c| c.is_ascii_digit())
                })
            })
            .ok_or_else(|| "未找到 Tweet ID（需要 >= 10 位的纯数字）".to_string())?;

        if tweet_id_index == 0 {
            return Err("Tweet ID 在文件名开头，无法确定用户名".to_string());
        }

        let tweet_id = tokens[tweet_id_index];
        let username = tokens[..tweet_id_index].join("_");

        Ok((username, tweet_id.to_string()))
    }

    fn handle_duplicate_file(source_path: &Path, target_path: &Path) -> Result<()> {
        let source_size = fs::metadata(source_path)?.len();
        let target_size = fs::metadata(target_path)?.len();

        println!(
            "发现同名文件: {}",
            source_path.file_name().unwrap().to_string_lossy()
        );
        println!("  源文件大小: {} 字节", source_size);
        println!("  目标文件大小: {} 字节", target_size);

        // 删除目标目录中的同名文件
        fs::remove_file(target_path)?;
        println!("  已删除目标目录中的同名文件: {:?}", target_path);

        Ok(())
    }
}

#[cfg(test)]
#[path = "organize_files_test.rs"]
mod tests;
