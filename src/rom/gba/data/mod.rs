//! GBA 数据类型（数据集）。

use crate::event::RomChecksum;

/// S29GL 系列统一扇区大小 128KB。
pub const SECTOR: u32 = 0x20000;

/// NOR flash 信息（CFI）。
pub struct FlashInfo {
    /// Autoselect ID（8 字节）；读不到为 None。
    pub id: Option<[u8; 8]>,
    pub device_size: u64,        // 字节
    pub buffer_write_bytes: u32, // 写缓冲大小(字节)，0=仅单字编程
    pub sector_size: u32,        // 统一扇区大小(字节)
    pub sector_count: u32,
}

impl FlashInfo {
    /// "01 02 ..." 形式的 ID；无则空串。
    pub fn id_hex(&self) -> String {
        match self.id {
            Some(id) => id
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" "),
            None => String::new(),
        }
    }
}

/// GBA ROM 头解析结果。
pub struct GbaHeader {
    pub rom_title: String,
    pub game_code: String,
    pub revision: u8,
    pub checksum: RomChecksum,
    pub game_name: String,
    pub rtc: bool,
}

/// 烧录选项。
pub struct BurnOptions {
    /// 整片擦除（否则逐扇区即擦即写）。
    pub chip_erase: bool,
    /// 开始前自动解锁 PPB。
    pub unlock_ppb: bool,
    /// 烧后校验 + 修复。
    pub verify: bool,
}

impl Default for BurnOptions {
    fn default() -> Self {
        Self { chip_erase: false, unlock_ppb: true, verify: true }
    }
}

/// 烧录结果。
pub struct BurnResult {
    pub success: bool,
    pub bytes_written: u64,
    pub reconnects: u32,
    pub first_bad: Option<u64>,
    pub mismatch_bytes: u32,
    pub seconds: f64,
}
