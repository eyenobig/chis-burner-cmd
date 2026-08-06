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
//! ⚠️ reconnect_as(gbc) 上 3.3V + gbc_warm_up；失败重连后应按偏好/默认确认 3.3V + 重新使能 RAM + 重选 bank。
//! ⚠️ 待硬件验证（见根目录 TODO.md）。
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
    match kind {
        MbcKind::Mbc1 => {
            link.gbc_write(0x6000, &[0x01]);
            link.gbc_write(0x4000, &[(bank & 0x03) as u8]);
        }
        MbcKind::Mbc2 => {}
        MbcKind::Mbc3 => {
            link.gbc_write(0x4000, &[(bank & 0x07) as u8]);
        }
        MbcKind::Mbc5 => {
            link.gbc_write(0x4000, &[(bank & 0xff) as u8]);
        }
    }
}

fn save_address(kind: MbcKind, ram_off: u64) -> (u32, u32) {
    if kind == MbcKind::Mbc2 {
        (0, RAM_WINDOW + (ram_off as u32 & 0x01ff))
    } else {
        ((ram_off >> 13) as u32, RAM_WINDOW + (ram_off as u32 & 0x1fff))
    }
}

fn validate_size(kind: MbcKind, len: u64, log: &mut dyn FnMut(&str)) -> bool {
    if kind == MbcKind::Mbc2 && len > 512 {
        log("MBC2 存档固定为 512 字节（每字节低 4 位有效）");
        false
    } else {
        true
    }
}

/// reconnect 后的复位：按电压偏好/默认重上电 + 重新使能 RAM。
fn rearm(link: &mut CartridgeLink, kind: MbcKind, bank: u32) {
    let _ = link.reconnect_as(true);
    crate::device::power(
        link,
        crate::device::voltage_for(crate::rom::common::CartridgeKind::GbMbc),
    );
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

/// 导出 `len` 字节存档到 `path`。调用前应已上电（默认 3.3V）+ `gbc_warm_up()`。
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
    if !validate_size(kind, len, log) {
        return fail(0, t0);
    }
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
        let (bank_u32, cart_addr) = save_address(kind, ram_off);
        let bank = bank_u32 as i64;
        if bank != current_bank {
            current_bank = bank;
            log(&crate::i18n::tf("save.bank", &[("n", &bank.to_string())]));
            switch_ram_bank(link, kind, bank as u32);
        }
        let b = &mut buf[..n];
        if !read_chunk(link, fram, cart_addr, b) {
            rearm(link, kind, bank as u32);
            current_bank = -1;
            continue;
        }
        if kind == MbcKind::Mbc2 {
            for byte in b.iter_mut() {
                *byte = (*byte & 0x0f) | 0xf0;
            }
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
    if !validate_size(kind, total, log) {
        return fail(0, t0);
    }

    ram_enable(link);
    let mut written = 0u64;
    let mut current_bank: i64 = -1;
    while written < total {
        let n = ((total - written) as usize).min(PACKET);
        let ram_off = written;
        let (bank_u32, cart_addr) = save_address(kind, ram_off);
        let bank = bank_u32 as i64;
        if bank != current_bank {
            current_bank = bank;
            log(&crate::i18n::tf("save.bank", &[("n", &bank.to_string())]));
            switch_ram_bank(link, kind, bank as u32);
        }
        let chunk = &data[written as usize..written as usize + n];
        if kind == MbcKind::Mbc2 {
            let low_nibbles: Vec<u8> = chunk.iter().map(|byte| byte & 0x0f).collect();
            write_chunk(link, fram, cart_addr, &low_nibbles);
        } else {
            write_chunk(link, fram, cart_addr, chunk);
        }
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
    if !validate_size(kind, total, log) {
        return fail(0, t0);
    }

    ram_enable(link);
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut mismatch: u32 = 0;
    let mut buf = vec![0u8; PACKET];
    while read < total {
        let n = ((total - read) as usize).min(PACKET);
        let ram_off = read;
        let (bank_u32, cart_addr) = save_address(kind, ram_off);
        let bank = bank_u32 as i64;
        if bank != current_bank {
            current_bank = bank;
            switch_ram_bank(link, kind, bank as u32);
        }
        let b = &mut buf[..n];
        if !read_chunk(link, fram, cart_addr, b) {
            rearm(link, kind, bank as u32);
            current_bank = -1;
            continue;
        }
        for i in 0..n {
            let expected = data[read as usize + i];
            let differs = if kind == MbcKind::Mbc2 {
                (expected & 0x0f) != (b[i] & 0x0f)
            } else {
                expected != b[i]
            };
            if differs {
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

