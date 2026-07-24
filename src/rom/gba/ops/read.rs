//! GBA · 读：flash ID/CFI、卡带在否、GBA 判别、ROM 头解析。
//!
//! flash 读取从 `GbaFlasher.cs ReadInfo()` 复刻；头解析对应 `mission_gba.cs`。

use crate::cartridge_link::CartridgeLink;
use crate::event::RomChecksum;
use crate::rom::common;
use crate::rom::gba::data::{FlashInfo, GbaHeader, SECTOR};

/// 读取 ID + CFI。**调用前应已上电 + `warm_up()`。**
///
/// 协议：先 `0xf0` 读 Autoselect ID(8B)；再对“字”地址 0x55 写 `0x98` 进 CFI 模式，
/// 一次性连续读 word 0x27..0x34，最后写 `0xf0` 复位回读阵列。CFI 字段按 JEDEC 解析。
pub fn read_info(link: &mut CartridgeLink) -> FlashInfo {
    let mut id_buf = [0u8; 8];
    let id = if link.rom_read_id(&mut id_buf) {
        Some(id_buf)
    } else {
        None
    };

    link.rom_write(0x55, &[0x98, 0x00]);
    let mut cfi = [0u8; (0x34 - 0x27 + 1) * 2]; // 覆盖 word 0x27..0x34
    link.rom_read(0x27 << 1, &mut cfi);
    link.rom_write(0x00, &[0xf0, 0x00]);

    let cfi_word = |w: usize| cfi[(w - 0x27) * 2] as u32;

    // 移位前夹住指数：无卡带时 CFI 全 0xFF，1<<255 在 debug 下会 panic、值也无意义。
    let size_exp = cfi_word(0x27);
    let device_size = if size_exp < 64 { 1u64 << size_exp } else { 0 };
    let buf_n = cfi_word(0x2a);
    let buffer_write_bytes = if (1..32).contains(&buf_n) { 1u32 << buf_n } else { 0 };
    let sector_count = ((cfi_word(0x2e) << 8) | cfi_word(0x2d)) + 1;
    let sector_size0 = ((cfi_word(0x30) << 8) | cfi_word(0x2f)) * 256;
    let sector_size = if sector_size0 > 0 { sector_size0 } else { SECTOR };

    FlashInfo {
        id,
        device_size,
        buffer_write_bytes,
        sector_size,
        sector_count,
    }
}

/// flash 芯片是否在位（有有效 CFI ID，且非全 0xFF 悬空）。
pub fn flash_present(flash: &FlashInfo) -> bool {
    flash.id.map_or(false, |id| !id.iter().all(|&b| b == 0xFF))
}

/// GBA 头补码校验：stored=header[0xBD]，computed = -(0x19 + Σ header[0xA0..=0xBC]) & 0xFF。
pub fn header_checksum(header: &[u8]) -> RomChecksum {
    if header.len() <= 0xBD {
        return RomChecksum { stored: 0, computed: 0, ok: false };
    }
    let stored = header[0xBD];
    let sum: u32 = header[0xA0..=0xBC].iter().map(|&b| b as u32).sum();
    let computed = (0u32.wrapping_sub(0x19u32.wrapping_add(sum)) & 0xFF) as u8;
    RomChecksum { stored, computed, ok: stored == computed }
}

/// 从 GBA 总线读到的头(≥0xC0 字节)判别是否 GBA：0xB2==0x96 且头校验通过、非空片。
pub fn is_gba_header(header: &[u8]) -> bool {
    header.len() >= 0xC0
        && !common::ops::is_blank(&header[..0xC0])
        && header[0xB2] == 0x96
        && header_checksum(header).ok
}

/// 已知带 RTC 的 GBA game code 前缀（启发式；真正的 GPIO/S3511 探测待移植）。
const RTC_PREFIXES: &[&str] = &[
    "AXV", "AXP", "BPE", // Pokémon Ruby / Sapphire / Emerald
    "U3I", "U32", "U33", // Boktai 1 / 2 / 3
    "BKA", "BR4",        // 千年家族 / 洛克人 EXE 4.5
];

/// 按 game code 启发式判断是否带 RTC。
pub fn has_rtc(game_code: &str) -> bool {
    let gc = game_code.to_uppercase();
    RTC_PREFIXES.iter().any(|p| gc.starts_with(p))
}

/// 取 ASCII 字段：遇 0 截断，非可打印字符替换为空格，去首尾空白。
fn ascii(header: &[u8], range: std::ops::Range<usize>) -> String {
    header[range]
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' })
        .collect::<String>()
        .trim()
        .to_string()
}

/// 解析 GBA 头（要求 header 至少 0xC0 字节；查游戏名需 0x180，不足则回退标题）。
pub fn parse_header(header: &[u8]) -> GbaHeader {
    let rom_title = ascii(header, 0xA0..0xAC);
    let game_code = ascii(header, 0xAC..0xB0);
    let revision = header.get(0xBC).copied().unwrap_or(0);
    let checksum = header_checksum(header);
    let rtc = has_rtc(&game_code);
    // GameName 与 RomTitle 同源（头内 0xA0..0xAC）；友好名由客户端 FlashRom API 另取。
    let game_name = rom_title.clone();

    GbaHeader {
        rom_title,
        game_code,
        revision,
        checksum,
        game_name,
        rtc,
    }
}
