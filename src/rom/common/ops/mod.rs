//! 通用操作（与平台无关）。

/// 一段字节是否全 0xFF（空 flash / 未写入 / 无数据）。
pub fn is_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(|&b| b == 0xFF)
}

/// GameName 解析。参考源无名称数据库，回退到 ROM 内部标题（预留按 game code 查表）。
pub fn game_name(rom_title: &str, _game_code: &str) -> String {
    rom_title.trim().to_string()
}
