use anyhow::Result;
use std::collections::HashMap;
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
                
                // 使用新的解析函数
                let parse_result = Self::parse_filename(&filename);
                
                match parse_result {
                    Ok((username, tweet_id)) => {
                        // 查找对应的目标文件夹
                        let mut target_folder = None;
                        for (folder_prefix, folder_path) in &b_folders {
                            if username == *folder_prefix 
                                || username.starts_with(&format!("{}_", folder_prefix))
                                || folder_prefix.starts_with(&format!("{}_", username)) {
                                target_folder = Some(folder_path);
                                break;
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
                                    println!("已将 {} 移动到 {:?} (用户名: {}, ID: {})", 
                                        filename, target_folder_path, username, tweet_id);
                                    moved_count += 1;
                                }
                                Err(e) => {
                                    println!("移动文件失败: {} -> {:?}, 错误: {}", 
                                        filename, destination, e);
                                    error_count += 1;
                                }
                            }
                        } else {
                            println!("未找到用户名为 {} 的目标文件夹，跳过 {}", username, filename);
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

    fn parse_filename(filename: &str) -> Result<(String, String), String> {
        let (filename_no_ext, _) = filename.rsplit_once('.').unwrap_or((filename, ""));
        let tokens: Vec<&str> = filename_no_ext.split('_').collect();
        
        if tokens.len() < 3 {
            return Err("文件名格式错误：至少需要3个部分 (a_b_c)".to_string());
        }
        
        // 找出所有纯数字的部分
        let numeric_indices: Vec<usize> = tokens.iter()
            .enumerate()
            .filter(|(_, token)| token.chars().all(|c| c.is_ascii_digit()))
            .map(|(i, _)| i)
            .collect();
        
        if numeric_indices.is_empty() {
            return Err("未找到数字ID部分".to_string());
        }
        
        // 选择最后一个数字作为Tweet ID
        let last_num_index = *numeric_indices.iter().max().unwrap();
        let tweet_id = tokens[last_num_index];
        
        // 根据最后一个数字的位置确定用户名和原始文件名
        let username = if last_num_index == 0 {
            // 格式：数字_用户名_原始文件名
            if tokens.len() >= 3 {
                tokens[1].to_string()
            } else {
                return Err("最后一个数字在开头但后面部分不足".to_string());
            }
        } else if last_num_index == tokens.len() - 1 {
            // 格式：用户名_原始文件名_数字
            if tokens.len() >= 3 {
                tokens[..last_num_index].join("_")
            } else {
                return Err("最后一个数字在结尾但前面部分不足".to_string());
            }
        } else {
            // 标准格式：用户名_数字_原始文件名
            tokens[..last_num_index].join("_")
        };
        
        Ok((username, tweet_id.to_string()))
    }

    fn handle_duplicate_file(source_path: &Path, target_path: &Path) -> Result<()> {
        let source_size = fs::metadata(source_path)?.len();
        let target_size = fs::metadata(target_path)?.len();
        
        println!("发现同名文件: {}", source_path.file_name().unwrap().to_string_lossy());
        println!("  源文件大小: {} 字节", source_size);
        println!("  目标文件大小: {} 字节", target_size);
        
        // 删除目标目录中的同名文件
        fs::remove_file(target_path)?;
        println!("  已删除目标目录中的同名文件: {:?}", target_path);
        
        Ok(())
    }
} 