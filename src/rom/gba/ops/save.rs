//! GBA · 存档：SRAM / FLASH / FRAM / 免电(batteryless) 的 dump / write / verify。
//!
//! 复刻自 `mission_gba.cs` 的 `mission_wrtieSram` / `mission_dumpRam` / `mission_verifyRam`
//! 及三个 `*_batteryless` 任务。低层用 `CartridgeLink` 的 `ram_*` / `rom_*` 原语。
//!
//! 约定（照搬 C#）：
//! - SRAM/FLASH/FRAM 走 GBA 字节地址，64KiB 一个 bank（`gba_sramSwitchBank` 写 word 0x800000）。
//! - 分块 4096B；FRAM latency = 25。
//! - FLASH 写前需整片擦除（JEDEC Chip-Erase 序列，轮询 ram_read(0)==0xff）。
//! - 免电存档藏在 ROM flash 里，靠魔数 `"<3 from Maniac"` 定位：见 [`batteryless_locate`]。
//!
//! ⚠️ GBA SRAM 路径已硬件验证；FLASH / FRAM / 免电及 `save-erase` 见根目录 TODO.md。
#![allow(dead_code)]

use std::time::Instant;

use crate::cartridge_link::CartridgeLink;

use super::delete::erase_sector;
use crate::rom::gba::data::{SaveResult, SaveType, SECTOR};

/// 分块大小（字节），与 C# 一致。
const PACKET: usize = 4096;
/// SRAM/FLASH/FRAM 一个 bank 64KiB。
const SRAM_BANK: u32 = 64 * 1024;
/// FRAM latency（GBA）。
const FRAM_LATENCY: u8 = 25;
/// 免电存档魔数（"Maniac" 家免电池补丁）。
const BATTERYLESS_MAGIC: &[u8] = b"<3 from Maniac";
/// 免电魔数后 0x0e 处的 payload_size 为 0 时的默认值。
const BATTERYLESS_PAYLOAD_DEFAULT: usize = 0x414;

/// 切 SRAM/FRAM bank（写 word 地址 0x800000，复刻 `gba_sramSwitchBank`）。
fn sram_switch_bank(link: &mut CartridgeLink, bank: u32) {
    link.rom_write(0x800000, &[(bank & 0xffff) as u8, ((bank >> 8) & 0xff) as u8]);
}

/// 切 FLASH 存档 bank（JEDEC bank-switch 序列，复刻 `gba_flashSwitchBank`）。
fn flash_switch_bank(link: &mut CartridgeLink, bank: u32) {
    let bank = if bank == 0 { 0 } else { 1 };
    link.ram_write(0x5555, &[0xaa]);
    link.ram_write(0x2aaa, &[0x55]);
    link.ram_write(0x5555, &[0xb0]);
    link.ram_write(0x0000, &[bank]);
}

/// FLASH 存档整片擦除：JEDEC Chip-Erase 序列后轮询 ram_read(0)==0xff。
/// 返回擦除的字节数（恒 0，仅用于结果统计占位语义）；false 表示超时未擦干净。
fn flash_chip_erase(link: &mut CartridgeLink, timeout_secs: u64) -> bool {
    link.ram_write(0x5555, &[0xaa]);
    link.ram_write(0x2aaa, &[0x55]);
    link.ram_write(0x5555, &[0x80]);
    link.ram_write(0x5555, &[0xaa]);
    link.ram_write(0x2aaa, &[0x55]);
    link.ram_write(0x5555, &[0x10]); // Chip Erase
    std::thread::sleep(std::time::Duration::from_millis(200));
    let start = Instant::now();
    let mut probe = [0u8; 1];
    loop {
        if link.ram_read(0x0000, &mut probe) && probe[0] == 0xff {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        if start.elapsed().as_secs() > timeout_secs {
            return false;
        }
    }
}

/// 按 save_type 切 bank（FLASH 用 flash 序列，其余用 sram 序列）。
fn switch_bank(link: &mut CartridgeLink, st: SaveType, bank: u32) {
    if matches!(st, SaveType::Flash) {
        flash_switch_bank(link, bank);
    } else {
        sram_switch_bank(link, bank);
    }
}

/// 写一个分块（按 save_type 分发到对应原语）。
fn write_chunk(link: &mut CartridgeLink, st: SaveType, base_addr: u32, chunk: &[u8]) {
    match st {
        SaveType::Flash => {
            link.ram_flash_program(base_addr, chunk);
        }
        SaveType::Fram => {
            link.ram_write_fram(base_addr, chunk, FRAM_LATENCY);
        }
        _ => {
            link.ram_write(base_addr, chunk);
        }
    }
}

/// 读一个分块（按 save_type 分发）。
fn read_chunk(link: &mut CartridgeLink, st: SaveType, base_addr: u32, out: &mut [u8]) -> bool {
    match st {
        SaveType::Fram => link.ram_read_fram(base_addr, out, FRAM_LATENCY),
        _ => link.ram_read(base_addr, out),
    }
}

/// 导出 `len` 字节存档到 `path`。
/// FLASH 类型也读得出来（无需擦除），故 dump 不区分擦除。
pub fn dump(
    link: &mut CartridgeLink,
    st: SaveType,
    len: u64,
    path: &str,
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let mut f = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(_) => {
            log(&crate::i18n::t("save.write_fail"));
            return fail(0, t0);
        }
    };
    use std::io::Write;

    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut buf = vec![0u8; PACKET];
    while read < len {
        let n = ((len - read) as usize).min(PACKET);
        let bank = (read / SRAM_BANK as u64) as u32;
        if bank as i64 != current_bank {
            current_bank = bank as i64;
            log(&crate::i18n::tf("save.bank", &[("n", &bank.to_string())]));
            switch_bank(link, st, bank);
        }
        let base_addr = (read & 0xffff) as u32;
        let b = &mut buf[..n];
        if !read_chunk(link, st, base_addr, b) {
            let _ = link.reconnect();
            current_bank = -1;
            continue;
        }
        if f.write_all(b).is_err() {
            log(&crate::i18n::t("save.write_fail"));
            return fail(read, t0);
        }
        read += n as u64;
        progress(read, len);
    }
    let _ = f.flush();
    ok(len, t0)
}

/// 写入存档（`data` 为 .sav 内容）。FLASH 会先整片擦除。
pub fn write(
    link: &mut CartridgeLink,
    st: SaveType,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let total = data.len() as u64;

    // FLASH 先整片擦除（JEDEC Chip-Erase + 轮询）。
    if matches!(st, SaveType::Flash) {
        log(&crate::i18n::t("save.erase"));
        if !flash_chip_erase(link, 30) {
            log(&crate::i18n::t("save.erase_fail"));
            return fail(0, t0);
        }
        progress(0, total);
    }

    let mut written = 0u64;
    let mut current_bank: i64 = -1;
    while written < total {
        let bank = (written / SRAM_BANK as u64) as u32;
        if bank as i64 != current_bank {
            current_bank = bank as i64;
            log(&crate::i18n::tf("save.bank", &[("n", &bank.to_string())]));
            switch_bank(link, st, bank);
        }
        let base_addr = (written & 0xffff) as u32;
        let n = ((total - written) as usize).min(PACKET);
        write_chunk(link, st, base_addr, &data[written as usize..written as usize + n]);
        written += n as u64;
        progress(written, total);
    }
    ok(total, t0)
}

/// 校验存档：逐字节比对 `data` 与卡内读出，返回不符字节数。
pub fn verify(
    link: &mut CartridgeLink,
    st: SaveType,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let total = data.len() as u64;

    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut mismatch: u32 = 0;
    let mut buf = vec![0u8; PACKET];
    while read < total {
        let bank = (read / SRAM_BANK as u64) as u32;
        if bank as i64 != current_bank {
            current_bank = bank as i64;
            switch_bank(link, st, bank);
        }
        let base_addr = (read & 0xffff) as u32;
        let n = ((total - read) as usize).min(PACKET);
        let b = &mut buf[..n];
        if !read_chunk(link, st, base_addr, b) {
            let _ = link.reconnect();
            current_bank = -1;
            continue;
        }
        for i in 0..n {
            if data[read as usize + i] != b[i] {
                mismatch += 1;
                if mismatch <= 32 {
                    log(&crate::i18n::tf(
                        "save.verify_mismatch",
                        &[
                            ("addr", &format!("0x{:08X}", read + i as u64)),
                            ("exp", &format!("{:02X}", data[read as usize + i])),
                            ("got", &format!("{:02X}", b[i])),
                        ],
                    ));
                }
            }
        }
        read += n as u64;
        progress(read, total);
    }
    SaveResult {
        success: mismatch == 0,
        bytes: total,
        mismatch_bytes: mismatch,
        seconds: t0.elapsed().as_secs_f64(),
    }
}

// EEPROM occupies the final 256 bytes of the 32 MiB Game Pak ROM window.
// CartridgeLink::rom_write uses halfword addresses; rom_read uses byte addresses.
const EEPROM_WORD_ADDR: u32 = 0x00ff_ff80;
const EEPROM_BYTE_ADDR: u32 = EEPROM_WORD_ADDR << 1;
const EEPROM_BLOCK: usize = 8;

fn eeprom_params(st: SaveType) -> Option<(usize, usize)> {
    match st {
        SaveType::Eeprom4k => Some((6, 512)),
        SaveType::Eeprom64k => Some((14, 8192)),
        _ => None,
    }
}

fn push_eeprom_bit(words: &mut Vec<u8>, bit: u8) {
    words.extend_from_slice(&(u16::from(bit & 1)).to_le_bytes());
}

fn push_eeprom_value(words: &mut Vec<u8>, value: u32, bits: usize) {
    for shift in (0..bits).rev() {
        push_eeprom_bit(words, ((value >> shift) & 1) as u8);
    }
}

fn eeprom_read_request(block: u32, address_bits: usize) -> Vec<u8> {
    let mut words = Vec::with_capacity((address_bits + 3) * 2);
    push_eeprom_bit(&mut words, 1);
    push_eeprom_bit(&mut words, 1);
    push_eeprom_value(&mut words, block, address_bits);
    push_eeprom_bit(&mut words, 0);
    words
}

fn eeprom_write_request(block: u32, address_bits: usize, data: &[u8; EEPROM_BLOCK]) -> Vec<u8> {
    let mut words = Vec::with_capacity((address_bits + 67) * 2);
    push_eeprom_bit(&mut words, 1);
    push_eeprom_bit(&mut words, 0);
    push_eeprom_value(&mut words, block, address_bits);
    for byte in data {
        push_eeprom_value(&mut words, u32::from(*byte), 8);
    }
    push_eeprom_bit(&mut words, 0);
    words
}

fn decode_eeprom_read(words: &[u8]) -> Option<[u8; EEPROM_BLOCK]> {
    if words.len() < (4 + 64) * 2 {
        return None;
    }
    let mut out = [0u8; EEPROM_BLOCK];
    for bit_index in 0..64 {
        let word_offset = (4 + bit_index) * 2;
        let bit = words[word_offset] & 1;
        out[bit_index / 8] = (out[bit_index / 8] << 1) | bit;
    }
    Some(out)
}

fn eeprom_read_block(link: &mut CartridgeLink, block: u32, address_bits: usize) -> Option<[u8; EEPROM_BLOCK]> {
    let request = eeprom_read_request(block, address_bits);
    if !link.rom_write(EEPROM_WORD_ADDR, &request) {
        return None;
    }
    let mut words = [0u8; (4 + 64) * 2];
    if !link.rom_read(EEPROM_BYTE_ADDR, &mut words) {
        return None;
    }
    decode_eeprom_read(&words)
}

fn eeprom_write_block(
    link: &mut CartridgeLink,
    block: u32,
    address_bits: usize,
    data: &[u8; EEPROM_BLOCK],
) -> bool {
    let request = eeprom_write_request(block, address_bits, data);
    if !link.rom_write(EEPROM_WORD_ADDR, &request) {
        return false;
    }

    let started = Instant::now();
    let mut ready = [0u8; 2];
    while started.elapsed() < std::time::Duration::from_millis(20) {
        if link.rom_read(EEPROM_BYTE_ADDR, &mut ready) && (ready[0] & 1) != 0 {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    false
}

fn validate_eeprom_size(st: SaveType, len: usize, log: &mut dyn FnMut(&str)) -> Option<(usize, usize)> {
    let (address_bits, expected) = eeprom_params(st)?;
    if len != expected {
        log(&format!("{} 存档必须正好为 {} 字节，实际为 {} 字节", st.label(), expected, len));
        None
    } else {
        Some((address_bits, expected))
    }
}

pub fn dump_eeprom(
    link: &mut CartridgeLink,
    st: SaveType,
    path: &str,
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let Some((address_bits, total)) = eeprom_params(st) else {
        return fail(0, t0);
    };
    let mut file = match std::fs::File::create(path) {
        Ok(file) => file,
        Err(_) => {
            log(&crate::i18n::t("save.write_fail"));
            return fail(0, t0);
        }
    };
    use std::io::Write;
    for offset in (0..total).step_by(EEPROM_BLOCK) {
        let Some(block) = eeprom_read_block(link, (offset / EEPROM_BLOCK) as u32, address_bits) else {
            log(&format!("EEPROM 读取失败 @ 0x{offset:04X}"));
            return fail(offset as u64, t0);
        };
        if file.write_all(&block).is_err() {
            log(&crate::i18n::t("save.write_fail"));
            return fail(offset as u64, t0);
        }
        progress((offset + EEPROM_BLOCK) as u64, total as u64);
    }
    let _ = file.flush();
    ok(total as u64, t0)
}

pub fn write_eeprom(
    link: &mut CartridgeLink,
    st: SaveType,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let Some((address_bits, total)) = validate_eeprom_size(st, data.len(), log) else {
        return fail(0, t0);
    };
    for offset in (0..total).step_by(EEPROM_BLOCK) {
        let block: &[u8; EEPROM_BLOCK] = data[offset..offset + EEPROM_BLOCK].try_into().unwrap();
        if !eeprom_write_block(link, (offset / EEPROM_BLOCK) as u32, address_bits, block) {
            log(&format!("EEPROM 写入超时 @ 0x{offset:04X}"));
            return fail(offset as u64, t0);
        }
        progress((offset + EEPROM_BLOCK) as u64, total as u64);
    }
    ok(total as u64, t0)
}

pub fn verify_eeprom(
    link: &mut CartridgeLink,
    st: SaveType,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let Some((address_bits, total)) = validate_eeprom_size(st, data.len(), log) else {
        return fail(0, t0);
    };
    let mut mismatch = 0u32;
    for offset in (0..total).step_by(EEPROM_BLOCK) {
        let Some(block) = eeprom_read_block(link, (offset / EEPROM_BLOCK) as u32, address_bits) else {
            log(&format!("EEPROM 读取失败 @ 0x{offset:04X}"));
            return fail(offset as u64, t0);
        };
        for i in 0..EEPROM_BLOCK {
            if data[offset + i] != block[i] {
                mismatch += 1;
                if mismatch <= 32 {
                    log(&format!(
                        "EEPROM 校验不符 @ 0x{:04X}: 期望 {:02X}，读到 {:02X}",
                        offset + i,
                        data[offset + i],
                        block[i]
                    ));
                }
            }
        }
        progress((offset + EEPROM_BLOCK) as u64, total as u64);
    }
    SaveResult {
        success: mismatch == 0,
        bytes: total as u64,
        mismatch_bytes: mismatch,
        seconds: t0.elapsed().as_secs_f64(),
    }
}

/// 定位免电存档在 ROM 镜像中的位置（纯函数，便于单测）。
///
/// 复刻 `gba_searchBatteryless`（单卡，base=0）：
/// 1. boot vector = `((u32_le@0 & 0xFFFFFF) + 2) << 2`
/// 2. 在 boot vector 起的 8KiB 窗口里搜魔数 `"<3 from Maniac"`
/// 3. 命中处 i：payload_size = u16_le@(i+0x0e)，为 0 则默认 0x414
///    存档数据绝对偏移 = boot_vector + i + 0x10
///    存档字节数 = u32_le@(存档偏移 - payload_size + 8)
///
/// 返回 `(save_offset, save_size)`；找不到返回 None。
pub fn batteryless_locate(rom: &[u8]) -> Option<(u64, u64)> {
    if rom.len() < 4 {
        return None;
    }
    let boot_vec_le = u32::from_le_bytes([rom[0], rom[1], rom[2], rom[3]]);
    let boot_vector = (((boot_vec_le & 0x00FF_FFFF) as u64) + 2) << 2;

    // 8KiB 搜索窗口。
    let win_start = boot_vector as usize;
    let win_end = win_start.checked_add(0x2000)?;
    if win_end > rom.len() {
        return None;
    }
    let win = &rom[win_start..win_end];

    // 找第一个魔数匹配（C# 找到即返回）。
    let i = win
        .windows(BATTERYLESS_MAGIC.len())
        .position(|w| w == BATTERYLESS_MAGIC)?;
    let abs_magic = win_start + i; // 魔数在 ROM 里的绝对偏移

    let payload_size = if i + 0x0e + 2 <= win.len() {
        let p = u16::from_le_bytes([win[i + 0x0e], win[i + 0x0f]]) as usize;
        if p == 0 { BATTERYLESS_PAYLOAD_DEFAULT } else { p }
    } else {
        BATTERYLESS_PAYLOAD_DEFAULT
    };

    let save_offset = (abs_magic + 0x10) as u64;
    let payload_start = save_offset as usize - payload_size;

    // 存档大小 = payload 头里偏移 8 的 u32。
    if payload_start + 8 + 4 > rom.len() {
        return None;
    }
    let save_size = u32::from_le_bytes([
        rom[payload_start + 8],
        rom[payload_start + 9],
        rom[payload_start + 10],
        rom[payload_start + 11],
    ]) as u64;

    Some((save_offset, save_size))
}

/// 免电存档 dump：先定位，再按 rom_read 读出。
pub fn dump_batteryless(
    link: &mut CartridgeLink,
    rom_mirror: &[u8],
    path: &str,
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let Some((offset, size)) = batteryless_locate(rom_mirror) else {
        log(&crate::i18n::t("save.not_found"));
        return fail(0, t0);
    };
    log(&crate::i18n::tf(
        "save.found",
        &[("off", &format!("0x{:08X}", offset)), ("size", &size.to_string())],
    ));

    let mut f = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(_) => {
            log(&crate::i18n::t("save.write_fail"));
            return fail(0, t0);
        }
    };
    use std::io::Write;

    let mut read = 0u64;
    let mut buf = vec![0u8; PACKET];
    while read < size {
        let n = ((size - read) as usize).min(PACKET);
        let addr = offset + read;
        let b = &mut buf[..n];
        if !link.rom_read(addr as u32, b) {
            let _ = link.reconnect();
            continue;
        }
        if f.write_all(b).is_err() {
            log(&crate::i18n::t("save.write_fail"));
            return fail(read, t0);
        }
        read += n as u64;
        progress(read, size);
    }
    let _ = f.flush();
    ok(size, t0)
}

/// 免电存档 write：定位 → 擦覆盖扇区 → rom_program 写入。
pub fn write_batteryless(
    link: &mut CartridgeLink,
    rom_mirror: &[u8],
    buffer_write_bytes: u16,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let Some((offset, mut size)) = batteryless_locate(rom_mirror) else {
        log(&crate::i18n::t("save.not_found"));
        return fail(0, t0);
    };
    if size > data.len() as u64 {
        size = data.len() as u64; // 按文件截断（复刻 C# 的 clamp）
    }
    log(&crate::i18n::tf(
        "save.found",
        &[("off", &format!("0x{:08X}", offset)), ("size", &size.to_string())],
    ));

    // 擦除覆盖存档区的各 128KB 扇区。
    log(&crate::i18n::tf(
        "save.erase_range",
        &[
            ("from", &format!("0x{:08X}", offset)),
            ("to", &format!("0x{:08X}", offset + size)),
        ],
    ));
    let mut sa = (offset / SECTOR as u64) * SECTOR as u64;
    while sa < offset + size {
        if !erase_sector(link, sa as u32, 3) {
            log(&crate::i18n::t("save.erase_fail"));
            return fail(0, t0);
        }
        sa += SECTOR as u64;
    }

    let mut written = 0u64;
    while written < size {
        let n = ((size - written) as usize).min(PACKET);
        let addr = offset + written;
        if !link.rom_program(addr as u32, &data[written as usize..written as usize + n], buffer_write_bytes) {
            let _ = link.reconnect();
            continue;
        }
        written += n as u64;
        progress(written, size);
    }
    ok(size, t0)
}

/// 免电存档 verify：定位 → 逐块读出比对。
pub fn verify_batteryless(
    link: &mut CartridgeLink,
    rom_mirror: &[u8],
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let Some((offset, mut size)) = batteryless_locate(rom_mirror) else {
        log(&crate::i18n::t("save.not_found"));
        return fail(0, t0);
    };
    if size > data.len() as u64 {
        size = data.len() as u64;
    }

    let mut read = 0u64;
    let mut mismatch: u32 = 0;
    let mut buf = vec![0u8; PACKET];
    while read < size {
        let n = ((size - read) as usize).min(PACKET);
        let addr = offset + read;
        let b = &mut buf[..n];
        if !link.rom_read(addr as u32, b) {
            let _ = link.reconnect();
            continue;
        }
        for i in 0..n {
            if data[read as usize + i] != b[i] {
                mismatch += 1;
                if mismatch <= 32 {
                    log(&crate::i18n::tf(
                        "save.verify_mismatch",
                        &[
                            ("addr", &format!("0x{:08X}", read + i as u64)),
                            ("exp", &format!("{:02X}", data[read as usize + i])),
                            ("got", &format!("{:02X}", b[i])),
                        ],
                    ));
                }
            }
        }
        read += n as u64;
        progress(read, size);
    }
    SaveResult {
        success: mismatch == 0,
        bytes: size,
        mismatch_bytes: mismatch,
        seconds: t0.elapsed().as_secs_f64(),
    }
}

fn ok(bytes: u64, t0: Instant) -> SaveResult {
    SaveResult { success: true, bytes, mismatch_bytes: 0, seconds: t0.elapsed().as_secs_f64() }
}

fn fail(bytes: u64, t0: Instant) -> SaveResult {
    SaveResult { success: false, bytes, mismatch_bytes: 0, seconds: t0.elapsed().as_secs_f64() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eeprom_command_lengths_match_gba_protocol() {
        let data = [0xA5; EEPROM_BLOCK];
        assert_eq!(eeprom_read_request(0x12, 6).len(), 9 * 2);
        assert_eq!(eeprom_read_request(0x1234, 14).len(), 17 * 2);
        assert_eq!(eeprom_write_request(0x12, 6, &data).len(), 73 * 2);
        assert_eq!(eeprom_write_request(0x1234, 14, &data).len(), 81 * 2);
    }

    #[test]
    fn eeprom_read_decoder_skips_four_dummy_bits_and_uses_msb_first() {
        let expected = [0x00, 0x01, 0x7f, 0x80, 0xa5, 0x5a, 0xfe, 0xff];
        let mut words = Vec::new();
        for _ in 0..4 {
            push_eeprom_bit(&mut words, 0);
        }
        for byte in expected {
            push_eeprom_value(&mut words, u32::from(byte), 8);
        }
        assert_eq!(decode_eeprom_read(&words), Some(expected));
    }

    /// 构造一个带免电存档的合成 ROM blob，魔数紧贴 boot_vector 之后。
    ///
    /// 布局（offset）：
    /// - 0..4:        boot vector 字段（解码后得到 boot_vector）
    /// - boot_vector: 魔数 "<3 from Maniac"（魔数相对 boot_vector 的下标 i=0）
    /// - +0x0e:       payload_size(u16)=0 → 用默认 0x414
    /// - +0x10:       存档数据
    /// payload 头位于 (存档偏移 - payload_default)，其偏移 8 处放存档字节数。
    /// 注意：boot_vector 要够大，使存档偏移 ≥ payload_default（payload 头落在正地址）。
    fn synth_batteryless_rom(save_bytes: &[u8]) -> Vec<u8> {
        let boot_vector: usize = 0x1000; // 远大于 payload_default(0x414)
        let le_val = ((boot_vector >> 2) - 2) as u32; // boot_vector = ((le+2)<<2)
        let magic_off = boot_vector; // 魔数相对 boot_vector 下标 i=0
        let save_off_abs = magic_off + 0x10;
        let payload_default = BATTERYLESS_PAYLOAD_DEFAULT;
        let payload_start = save_off_abs - payload_default;

        // 需容纳 8KiB 搜索窗口（boot_vector..boot_vector+0x2000）+ payload 头 + 存档数据。
        let win_end = boot_vector + 0x2000;
        let need = (payload_start + 12 + save_bytes.len())
            .max(save_off_abs + save_bytes.len())
            .max(win_end);
        let mut rom = vec![0u8; need];
        rom[0..4].copy_from_slice(&le_val.to_le_bytes());
        rom[magic_off..magic_off + BATTERYLESS_MAGIC.len()].copy_from_slice(BATTERYLESS_MAGIC);
        // payload_size@+0x0e 留 0 → 用默认。
        rom[payload_start + 8..payload_start + 12].copy_from_slice(&(save_bytes.len() as u32).to_le_bytes());
        rom[save_off_abs..save_off_abs + save_bytes.len()].copy_from_slice(save_bytes);
        rom
    }

    #[test]
    fn batteryless_locate_finds_save() {
        let save = [0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03];
        let rom = synth_batteryless_rom(&save);
        let (off, size) = batteryless_locate(&rom).expect("应定位到免电存档");
        assert_eq!(size, save.len() as u64);
        // 存档内容应与写入一致。
        assert_eq!(&rom[off as usize..off as usize + save.len()], &save[..]);
    }

    #[test]
    fn batteryless_locate_missing_magic() {
        let rom = vec![0u8; 0x2000];
        assert!(batteryless_locate(&rom).is_none());
    }
}
