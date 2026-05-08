use crate::error::ErrorPayload;
use crate::setup;

use super::types::ImportOutput;

/// 从 cURL 文本（字符串）导入凭据 + 协议参数。
///
/// 安全：cURL 文本含敏感凭据（cookies / bearer / auth headers）；本函数
/// **不写任何临时文件**，整个解析过程在内存中完成，仅最终的解析结果
/// （凭据 KV 字段）落盘到 `credentials_path()` 解析出的稳定位置。
pub fn import_curl(curl_text: &str) -> Result<ImportOutput, ErrorPayload> {
    setup::run_setup_from_text(curl_text, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 关键安全断言：调用 import_curl 后，固定的 /tmp/xld_import_curl.txt 不应存在。
    /// 即使 import_curl 因为参数错误失败，也禁止留下任何临时文件痕迹。
    #[test]
    fn import_curl_does_not_create_predictable_temp_file() {
        let predictable = std::env::temp_dir().join("xld_import_curl.txt");
        // 先确保该路径干净（如果存在，删掉以保证测试初始状态）
        let _ = std::fs::remove_file(&predictable);

        // 故意用一段无效 cURL，让函数走错误路径
        let bogus = "not a curl command";
        let result = import_curl(bogus);
        assert!(result.is_err(), "无效 cURL 应当返回错误");

        // 关键断言：预期的临时文件路径不应被创建
        assert!(
            !predictable.exists(),
            "import_curl 必须不创建可预测的临时文件，但发现 {:?}",
            predictable
        );
    }
}
