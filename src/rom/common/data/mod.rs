//! 通用数据类型（与具体平台无关）。

/// 卡带类型判别结果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CartridgeKind {
    Gba,
    /// GB/GBC（MBC）——通过 GB 总线协议读取。
    #[allow(dead_code)]
    GbMbc,
    Unknown,
}

impl CartridgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CartridgeKind::Gba => "gba",
            CartridgeKind::GbMbc => "gb_mbc",
            CartridgeKind::Unknown => "unknown",
        }
    }
}
