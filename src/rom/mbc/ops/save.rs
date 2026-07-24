//! MBC · 存档：SRAM / FRAM 的 dump / write / verify。
//!
//! 复刻自 `mission_mbc5.cs` 的 `mission_wrtieRam_mbc5` / `mission_dumpRam_mbc5` /
//! `mission_verifyRam_mbc5`（也覆盖 MBC3，按 kind 分发 bank 掩码）。
//!
//! 协议要点（GB 总线，已用现有 `gbc_read`/`gbc_write` 原语）：
//! - RAM 使能：`gbc_write(0x0000, 0x0A)`
//! - RAM bank 切换：`gbc_write(0x4000, bank)`（MBC3 取低 3 位，MBC5 取低 8 位）
//! - 8KiB 一个 bank：bank = ram_off >> 13，窗口 = 0xA000 + (ram_off & 0x1fff)
//! - FRAM 用 `gbc_*_fram`，latency = 10
//!
//! ⚠️ reconnect 会把电压拉回 3.3V，故失败重连后需重上 5V + 重新使能 RAM + 重选 bank。
//! ⚠️ 未经硬件测试（见根目录 TODO.md）。
#![allow(dead_code)]

use std::time::Instant;

use crate::cartridge_link::CartridgeLink;
use crate::rom::mbc::data::MbcKind;

use crate::rom::gba::data::SaveResult;

/// 分块大小（字节）。
const PACKET: usize = 4096;
/// 8KiB RAM bank。
const RAM_BANK: u64 = 0x2000;
/// RAM 窗口基址。
const RAM_WINDOW: u32 = 0xA000;
/// MBC FRAM latency。
const FRAM_LATENCY: u8 = 10;

/// 使能卡带 RAM（写 0x0A 到 0x0000）。
fn ram_enable(link: &mut CartridgeLink) {
    link.gbc_write(0x0000, &[0x0A]);
}

/// 选 RAM bank（写 0x4000，按 MBC 代次取掩码）。复刻 `mbc_ramSwitchBank`。
fn switch_ram_bank(link: &mut CartridgeLink, kind: MbcKind, bank: u32) {
    let b = match kind {
        MbcKind::Mbc3 => (bank & 0x07) as u8,
        MbcKind::Mbc5 => (bank & 0xff) as u8,
    };
    link.gbc_write(0x4000, &[b]);
}

/// reconnect 后的复位：重上 5V + 重新使能 RAM。
fn rearm(link: &mut CartridgeLink, kind: MbcKind, bank: u32) {
    let _ = link.reconnect_as(true);
    ram_enable(link);
    switch_ram_bank(link, kind, bank);
}

fn write_chunk(link: &mut CartridgeLink, fram: bool, cart_addr: u32, chunk: &[u8]) {
    if fram {
        link.gbc_write_fram(cart_addr, chunk, FRAM_LATENCY);
    } else {
        link.gbc_write(cart_addr, chunk);
    }
}

fn read_chunk(link: &mut CartridgeLink, fram: bool, cart_addr: u32, out: &mut [u8]) -> bool {
    if fram {
        link.gbc_read_fram(cart_addr, out, FRAM_LATENCY)
    } else {
        link.gbc_read(cart_addr, out)
    }
}

/// 导出 `len` 字节存档到 `path`。调用前应已上电（5V 视卡而定）+ `gbc_warm_up()`。
pub fn dump(
    link: &mut CartridgeLink,
    kind: MbcKind,
    fram: bool,
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

    ram_enable(link);
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut buf = vec![0u8; PACKET];
    while read < len {
        let n = ((len - read) as usize).min(PACKET);
        let ram_off = read;
        let bank = (ram_off >> 13) as i64;
        if bank != current_bank {
            current_bank = bank;
            log(&crate::i18n::tf("save.bank", &[("n", &bank.to_string())]));
            switch_ram_bank(link, kind, bank as u32);
        }
        let cart_addr = RAM_WINDOW + (ram_off as u32 & 0x1fff);
        let b = &mut buf[..n];
        if !read_chunk(link, fram, cart_addr, b) {
            rearm(link, kind, bank as u32);
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

/// 写入存档（`data` 为 .sav 内容）。SRAM/FRAM 可直接写，无需显式擦除。
pub fn write(
    link: &mut CartridgeLink,
    kind: MbcKind,
    fram: bool,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let total = data.len() as u64;

    ram_enable(link);
    let mut written = 0u64;
    let mut current_bank: i64 = -1;
    while written < total {
        let n = ((total - written) as usize).min(PACKET);
        let ram_off = written;
        let bank = (ram_off >> 13) as i64;
        if bank != current_bank {
            current_bank = bank;
            log(&crate::i18n::tf("save.bank", &[("n", &bank.to_string())]));
            switch_ram_bank(link, kind, bank as u32);
        }
        let cart_addr = RAM_WINDOW + (ram_off as u32 & 0x1fff);
        write_chunk(link, fram, cart_addr, &data[written as usize..written as usize + n]);
        written += n as u64;
        progress(written, total);
    }
    ok(total, t0)
}

/// 校验存档：逐字节比对 `data` 与卡内读出。
pub fn verify(
    link: &mut CartridgeLink,
    kind: MbcKind,
    fram: bool,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let total = data.len() as u64;

    ram_enable(link);
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut mismatch: u32 = 0;
    let mut buf = vec![0u8; PACKET];
    while read < total {
        let n = ((total - read) as usize).min(PACKET);
        let ram_off = read;
        let bank = (ram_off >> 13) as i64;
        if bank != current_bank {
            current_bank = bank;
            switch_ram_bank(link, kind, bank as u32);
        }
        let cart_addr = RAM_WINDOW + (ram_off as u32 & 0x1fff);
        let b = &mut buf[..n];
        if !read_chunk(link, fram, cart_addr, b) {
            rearm(link, kind, bank as u32);
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

fn ok(bytes: u64, t0: Instant) -> SaveResult {
    SaveResult { success: true, bytes, mismatch_bytes: 0, seconds: t0.elapsed().as_secs_f64() }
}

fn fail(bytes: u64, t0: Instant) -> SaveResult {
    SaveResult { success: false, bytes, mismatch_bytes: 0, seconds: t0.elapsed().as_secs_f64() }
}

/// 读 ROM 跨度 `[offset, offset+len)`（线性 ROM 偏移，按 MBC bank 切换）。
fn read_rom_span(
    link: &mut CartridgeLink,
    kind: MbcKind,
    offset: u64,
    len: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Option<Vec<u8>> {
    use super::read::{bus_addr, switch_bank};
    let mut out = vec![0u8; len as usize];
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut buf = vec![0u8; PACKET];
    while read < len {
        let n = ((len - read) as usize).min(PACKET);
        let rom_off = (offset + read) as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            current_bank = bank;
            switch_bank(link, bank as u32, kind);
        }
        let cartridge_addr = bus_addr(rom_off, kind);
        let b = &mut buf[..n];
        if !link.gbc_read(cartridge_addr, b) {
            let _ = link.reconnect();
            crate::device::power(link, crate::device::data::Voltage::V5);
            link.gbc_warm_up();
            switch_bank(link, bank as u32, kind);
            continue;
        }
        out[read as usize..read as usize + n].copy_from_slice(b);
        read += n as u64;
        progress(read, len);
    }
    Some(out)
}

/// 免电存档 dump：按 `db_DMG_bl` 的 offset/size/layout 从 ROM flash 抽出 .sav。
pub fn dump_batteryless(
    link: &mut CartridgeLink,
    kind: MbcKind,
    cfg: &crate::gamedb::BatterylessConfig,
    path: &str,
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    log(&crate::i18n::tf(
        "save.found",
        &[
            ("off", &format!("0x{:08X}", cfg.offset)),
            ("size", &cfg.size.to_string()),
        ],
    ));
    log(&format!(
        "免电布局 layout={} rom_span={}",
        cfg.layout,
        cfg.rom_span()
    ));
    let Some(span) = read_rom_span(link, kind, cfg.offset, cfg.rom_span(), progress) else {
        return fail(0, t0);
    };
    let save = crate::gamedb::extract_batteryless_save(&span, cfg);
    if std::fs::write(path, &save).is_err() {
        log(&crate::i18n::t("save.write_fail"));
        return fail(0, t0);
    }
    ok(save.len() as u64, t0)
}

/// 免电存档 write：把 .sav 按 layout 展开后擦扇区并烧进 ROM flash。
pub fn write_batteryless(
    link: &mut CartridgeLink,
    kind: MbcKind,
    cfg: &crate::gamedb::BatterylessConfig,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    log(&crate::i18n::tf(
        "save.found",
        &[
            ("off", &format!("0x{:08X}", cfg.offset)),
            ("size", &cfg.size.to_string()),
        ],
    ));
    let image = crate::gamedb::expand_batteryless_image(data, cfg);
    let end = cfg.offset + image.len() as u64;
    log("擦除免电存档区 ...");
    let sector_size = super::read::rom_get_cfi(link).2;
    if !super::delete::erase_range(link, kind, cfg.offset, end, sector_size) {
        log("擦除失败");
        return fail(0, t0);
    }
    log("写入免电存档 ...");
    if let Some(bad) = super::write::program_range(link, kind, &image, cfg.offset, progress) {
        log(&format!("写入失败 @ 0x{bad:08X}"));
        return fail(bad.saturating_sub(cfg.offset), t0);
    }
    ok(cfg.size.min(data.len() as u64), t0)
}

/// 免电存档 verify：读出并与 .sav 比对。
pub fn verify_batteryless(
    link: &mut CartridgeLink,
    kind: MbcKind,
    cfg: &crate::gamedb::BatterylessConfig,
    data: &[u8],
    log: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> SaveResult {
    let t0 = Instant::now();
    let Some(span) = read_rom_span(link, kind, cfg.offset, cfg.rom_span(), progress) else {
        return fail(0, t0);
    };
    let got = crate::gamedb::extract_batteryless_save(&span, cfg);
    let n = data.len().min(got.len()).min(cfg.size as usize);
    let mut mismatch = 0u32;
    for i in 0..n {
        if data[i] != got[i] {
            mismatch += 1;
            if mismatch <= 32 {
                log(&crate::i18n::tf(
                    "save.verify_mismatch",
                    &[
                        ("addr", &format!("0x{:08X}", i)),
                        ("exp", &format!("{:02X}", data[i])),
                        ("got", &format!("{:02X}", got[i])),
                    ],
                ));
            }
        }
    }
    SaveResult {
        success: mismatch == 0,
        bytes: n as u64,
        mismatch_bytes: mismatch,
        seconds: t0.elapsed().as_secs_f64(),
    }
}
