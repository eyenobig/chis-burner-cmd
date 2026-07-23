//! 通用操作（与平台无关）。

/// 一段字节是否全 0xFF（空 flash / 未写入 / 无数据）。
pub fn is_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(|&b| b == 0xFF)
}
