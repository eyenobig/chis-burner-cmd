//! ROM 操作：按平台分 `gba` / `mbc`，跨平台通用部分在 `common`。
//! 每个平台子模块再拆 `data`(数据集) + `ops`(实现函数)，与 `device` 同构。
//! 本文件做卡带信息的编排 + `cfb info` 命令。

pub mod common;
pub mod gba;
pub mod mbc;

use std::process::ExitCode;

use crate::cartridge_link::{CartridgeLink, USB_PID, USB_VID};
use crate::event::{emit, Event};
use crate::{device, i18n};

use common::CartridgeKind;
use gba::data::BurnOptions;
use gba::{FlashInfo, GbaHeader};
use mbc::data::MbcHeader;

/// `cfb info` —— 读 flash 芯片 + 卡带/游戏信息（人类可读 / `--json`）。
pub fn cmd_info(json: bool, port: Option<String>, mbc: bool) -> ExitCode {
    let port = match device::resolve_port(port) {
        Some(p) => p,
        None => {
            if json {
                emit(&Event::Error {
                    command: "info".to_string(),
                    message: i18n::tf(
                        "err.no_burner",
                        &[("vid", &format!("{USB_VID:04X}")), ("pid", &format!("{USB_PID:04X}"))],
                    ),
                });
            }
            return ExitCode::from(2);
        }
    };

    let mut link = CartridgeLink::new(&port);
    if let Err(e) = link.open() {
        emit_or_eprint(json, &i18n::tf("info.err_open", &[("port", &port), ("err", &e.to_string())]));
        return ExitCode::from(3);
    }

    device::power(&mut link, device::voltage_for(if mbc { CartridgeKind::GbMbc } else { CartridgeKind::Gba }));
    if mbc {
        link.gbc_warm_up();
        let header = mbc::ops::read::read_live_header(&mut link);
        let (capacity, buffer) = mbc::ops::read::rom_get_size(&mut link);
        device::power_off(&mut link);
        let Some(header) = header else {
            emit_or_eprint(json, &i18n::t("info.err_read"));
            return ExitCode::from(3);
        };
        if json {
            emit_mbc_info(&port, capacity, buffer, &header);
        } else {
            print_mbc_human(Some(&port), capacity, Some(buffer), &header);
        }
        return ExitCode::SUCCESS;
    }

    link.warm_up();
    let flash = gba::ops::read_info(&mut link);
    let present = gba::ops::flash_present(&flash);

    // flash 在位才有必要读 GBA 总线头做判别/解析。
    let header = if present {
        let mut h = [0u8; 0xC0];
        if link.rom_read(0, &mut h) {
            Some(h)
        } else {
            None
        }
    } else {
        None
    };
    device::power_off(&mut link);

    if !present {
        emit_or_eprint(json, &i18n::t("info.no_cartridge"));
        return ExitCode::from(3);
    }

    let kind = match header.as_ref() {
        Some(h) if gba::ops::is_gba_header(h) => CartridgeKind::Gba,
        _ => CartridgeKind::Unknown,
    };
    let game: Option<GbaHeader> = match (kind, header.as_ref()) {
        (CartridgeKind::Gba, Some(h)) => Some(gba::ops::parse_header(h)),
        _ => None,
    };

    if json {
        emit_gba_info(&port, kind.as_str(), &flash.id_hex(), flash.device_size, flash.buffer_write_bytes, flash.sector_size, flash.sector_count, game.as_ref());
    } else {
        print_human(&port, kind, &flash, game.as_ref());
    }
    ExitCode::SUCCESS
}

/// 发出一条 GBA 侧 `info` 事件（`cmd_info` 实时读 / `cmd_rom_info` 离线解析共用）。
/// `kind` 实时读时可能是 `"unknown"`（flash 在位但头部未识别）；离线解析恒 `"gba"`。
/// `game` 为 `None` 时游戏字段全部为 `null`。
fn emit_gba_info(
    port: &str,
    kind: &str,
    id: &str,
    capacity_bytes: u64,
    buffer_write_bytes: u32,
    sector_size: u32,
    sector_count: u32,
    game: Option<&GbaHeader>,
) {
    emit(&Event::Info {
        port: port.to_string(),
        present: true,
        kind: kind.to_string(),
        id: id.to_string(),
        capacity_bytes,
        buffer_write_bytes,
        sector_size,
        sector_count,
        game_name: game.map(|g| g.game_name.clone()),
        rom_title: game.map(|g| g.rom_title.clone()),
        game_code: game.map(|g| g.game_code.clone()),
        revision: game.map(|g| g.revision),
        rom_checksum: game.map(|g| g.checksum.clone()),
        rtc: game.map(|g| g.rtc),
    });
}

/// 发出一条 `gb_mbc` 类 `info` 事件（`cmd_info` 的 live 读 / `cmd_rom_info` 离线解析共用）。
/// `capacity`/`buffer` 传 0 表示离线场景（无 flash CFI 数据），回落用头部 `rom_size_bytes`。
fn emit_mbc_info(port: &str, capacity: u64, buffer: u16, h: &MbcHeader) {
    emit(&Event::Info {
        port: port.to_string(),
        present: true,
        kind: "gb_mbc".to_string(),
        id: String::new(),
        capacity_bytes: if capacity == 0 { h.rom_size_bytes } else { capacity },
        buffer_write_bytes: buffer as u32,
        sector_size: 0,
        sector_count: 0,
        game_name: Some(h.title.clone()),
        rom_title: Some(h.title.clone()),
        game_code: None,
        revision: None,
        rom_checksum: Some(h.header_checksum.clone()),
        rtc: Some(h.rtc),
    });
}

fn emit_or_eprint(json: bool, msg: &str) {
    if json {
        emit(&Event::Error {
            command: "info".to_string(),
            message: msg.to_string(),
        });
    } else {
        eprintln!("{msg}");
    }
}

fn yn(b: bool) -> String {
    i18n::t(if b { "common.yes" } else { "common.no" })
}

fn print_human(port: &str, kind: CartridgeKind, flash: &FlashInfo, game: Option<&GbaHeader>) {
    let kind_label = i18n::t(match kind {
        CartridgeKind::Gba => "kind.gba",
        CartridgeKind::GbMbc => "kind.gb_mbc",
        CartridgeKind::Unknown => "kind.unknown",
    });

    println!("{}", i18n::tf("info.port", &[("port", port)]));
    println!("{}", i18n::tf("info.cartridge", &[("kind", &kind_label)]));
    println!("{}", i18n::tf("info.id", &[("id", &flash.id_hex())]));
    let mb = flash.device_size / 1024 / 1024;
    println!(
        "{}",
        i18n::tf("info.capacity", &[("bytes", &flash.device_size.to_string()), ("mb", &mb.to_string())])
    );
    println!("{}", i18n::tf("info.buffer", &[("n", &flash.buffer_write_bytes.to_string())]));
    println!(
        "{}",
        i18n::tf("info.sector", &[("size", &flash.sector_size.to_string()), ("count", &flash.sector_count.to_string())])
    );

    match game {
        Some(g) => {
            println!("{}", i18n::t("info.sec_game"));
            println!("{}", i18n::tf("info.game_name", &[("name", &g.game_name)]));
            println!("{}", i18n::tf("info.rom_title", &[("title", &g.rom_title)]));
            println!("{}", i18n::tf("info.game_code", &[("code", &g.game_code)]));
            println!("{}", i18n::tf("info.revision", &[("rev", &g.revision.to_string())]));
            println!("{}", i18n::tf("info.rtc", &[("yn", &yn(g.rtc))]));
            print_checksum(&g.checksum);
        }
        None => println!("{}", i18n::t("info.no_game")),
    }
}

fn print_mbc_human(port: Option<&str>, capacity: u64, buffer: Option<u16>, h: &MbcHeader) {
    if let Some(port) = port {
        println!("{}", i18n::tf("info.port", &[("port", port)]));
        println!("{}", i18n::tf("info.cartridge", &[("kind", &i18n::t("kind.gb_mbc"))]));
    }
    println!("{}", i18n::tf("info.rom_title", &[("title", &h.title)]));
    println!("{}", i18n::tf("info.mbc_type", &[("name", h.mbc_name), ("code", &format!("{:02X}", h.cartridge_type))]));
    println!("{}", i18n::tf("info.cgb", &[("flag", &format!("{:02X}", h.cgb_flag))]));
    let size = if capacity == 0 { h.rom_size_bytes } else { capacity };
    println!("{}", i18n::tf("info.capacity", &[("bytes", &size.to_string()), ("mb", &(size / 1024 / 1024).to_string())]));
    if let Some(n) = buffer {
        println!("{}", i18n::tf("info.buffer", &[("n", &n.to_string())]));
    }
    println!("{}", i18n::tf("info.rtc", &[("yn", &yn(h.rtc))]));
    print_checksum(&h.header_checksum);
}

fn print_checksum(c: &crate::event::RomChecksum) {
    if c.ok {
        println!("{}", i18n::tf("info.checksum_ok", &[("stored", &format!("{:02X}", c.stored))]));
    } else {
        println!("{}", i18n::tf("info.checksum_bad", &[("stored", &format!("{:02X}", c.stored)), ("computed", &format!("{:02X}", c.computed))]));
    }
}

/// `cfb rom-info --file <path>` —— 离线解析本地 ROM 文件头，不需要烧录器。
pub fn cmd_rom_info(json: bool, path: &str) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            if json {
                emit(&Event::Error { command: "rom-info".to_string(), message: e.to_string() });
            } else {
                eprintln!("{e}");
            }
            return ExitCode::from(1);
        }
    };

    // 判别 GBA（头部至少 0xC0 字节，且通过 GBA 头校验）vs GB/GBC
    if bytes.len() >= 0xC0 && gba::ops::is_gba_header(&bytes) {
        let h = gba::ops::parse_header(&bytes);
        if json {
            emit_gba_info("", "gba", "", bytes.len() as u64, 0, 0, 0, Some(&h));
        } else {
            println!("{}", i18n::tf("rom_info.file", &[("path", path)]));
            println!("{}", i18n::tf("info.cartridge", &[("kind", &i18n::t("kind.gba"))]));
            println!("{}", i18n::tf("info.rom_title", &[("title", &h.rom_title)]));
            println!("{}", i18n::tf("info.game_code", &[("code", &h.game_code)]));
            println!("{}", i18n::tf("info.revision", &[("rev", &h.revision.to_string())]));
            println!("{}", i18n::tf("info.capacity", &[("bytes", &bytes.len().to_string()), ("mb", &(bytes.len() / 1024 / 1024).to_string())]));
            println!("{}", i18n::tf("info.rtc", &[("yn", &yn(h.rtc))]));
            print_checksum(&h.checksum);
        }
    } else if bytes.len() >= 0x150 {
        let h = mbc::ops::read::parse_header(&bytes);
        if json {
            emit_mbc_info("", 0, 0, &h);
        } else {
            println!("{}", i18n::tf("rom_info.file", &[("path", path)]));
            println!("{}", i18n::tf("info.cartridge", &[("kind", &i18n::t("kind.gb_mbc"))]));
            print_mbc_human(None, h.rom_size_bytes, None, &h);
        }
    } else {
        if json {
            emit(&Event::Error { command: "rom-info".to_string(), message: "file too small or unrecognized format".to_string() });
        } else {
            eprintln!("{}", i18n::t("rom_info.unrecognized"));
        }
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

// ==================== 写 / 删 / 导 命令 ====================

/// 打开端口并按卡型上电（GBA=3.3V / MBC=5V，受 `voltage` 偏好覆盖）。GBA 额外 warm_up。
fn open_powered(json: bool, cmd: &str, port: Option<String>, mbc: bool) -> Option<CartridgeLink> {
    let port = match device::resolve_port(port) {
        Some(p) => p,
        None => {
            if json {
                emit(&Event::Error {
                    command: cmd.to_string(),
                    message: i18n::tf(
                        "err.no_burner",
                        &[("vid", &format!("{USB_VID:04X}")), ("pid", &format!("{USB_PID:04X}"))],
                    ),
                });
            }
            return None;
        }
    };
    let mut link = CartridgeLink::new(&port);
    if let Err(e) = link.open() {
        op_err(json, cmd, &i18n::tf("info.err_open", &[("port", &port), ("err", &e.to_string())]));
        return None;
    }
    let kind = if mbc { CartridgeKind::GbMbc } else { CartridgeKind::Gba };
    device::power(&mut link, device::voltage_for(kind));
    if !mbc {
        link.warm_up();
    } else {
        link.gbc_warm_up(); // MBC：GB 总线版，吸收上电首包 + flash 复位
    }
    Some(link)
}

fn op_err(json: bool, cmd: &str, msg: &str) {
    if json {
        emit(&Event::Error { command: cmd.to_string(), message: msg.to_string() });
    } else {
        eprintln!("{msg}");
    }
}

fn log_emit(json: bool, m: &str) {
    if json {
        emit(&Event::Log { message: m.to_string() });
    } else {
        println!("{m}");
    }
}

fn progress_emit(json: bool, done: u64, total: u64, last_mb: &mut u64) {
    if json {
        emit(&Event::Progress { done, total });
    } else {
        let mb = done / (1 << 20);
        if mb != *last_mb {
            *last_mb = mb;
            println!("  {} / {} MB", mb, total / (1 << 20));
        }
    }
}

fn finish(json: bool, cmd: &str, ok: bool, bytes: u64, mm: u32, secs: f64) -> ExitCode {
    if json {
        emit(&Event::Result {
            command: cmd.to_string(),
            ok,
            bytes,
            mismatch_bytes: mm,
            seconds: secs,
        });
    } else {
        let key = if ok { "op.ok" } else { "op.fail" };
        println!(
            "{}",
            i18n::tf(
                key,
                &[("cmd", cmd), ("bytes", &bytes.to_string()), ("mm", &mm.to_string()), ("s", &format!("{secs:.0}"))]
            )
        );
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

/// `cfb burn --rom <f> [--mbc]` —— 写入 ROM。
pub fn cmd_burn(
    json: bool,
    port: Option<String>,
    rom_path: &str,
    mbc: bool,
    chip_erase: bool,
    unlock_ppb: bool,
    verify: bool,
) -> ExitCode {
    let data = match std::fs::read(rom_path) {
        Ok(d) => d,
        Err(e) => {
            op_err(json, "burn", &i18n::tf("op.read_fail", &[("path", rom_path), ("err", &e.to_string())]));
            return ExitCode::from(2);
        }
    };
    let Some(mut link) = open_powered(json, "burn", port, mbc) else {
        return ExitCode::from(3);
    };

    let mut last_mb = u64::MAX;
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let mut log = |m: &str| log_emit(json, m);

    let res = if mbc {
        mbc::ops::write::burn(&mut link, &data, verify, &mut progress, &mut log)
    } else {
        let opt = BurnOptions { chip_erase, unlock_ppb, verify };
        gba::ops::write::burn(&mut link, &data, &opt, &mut progress, &mut log)
    };
    device::power_off(&mut link);
    finish(json, "burn", res.success, res.bytes_written, res.mismatch_bytes, res.seconds)
}

/// `cfb rtc [--mbc]` —— 读取卡带 RTC 时间。
pub fn cmd_rtc_read(json: bool, port: Option<String>, mbc: bool) -> ExitCode {
    let Some(mut link) = open_powered(json, "rtc", port, mbc) else {
        return ExitCode::from(3);
    };

    if mbc {
        match mbc::ops::rtc::read_mbc3_rtc(&mut link) {
            Some(t) => {
                device::power_off(&mut link);
                if json {
                    emit(&Event::RtcData {
                        ok: true,
                        kind: "mbc3".to_string(),
                        year: None, month: None, date: None, day_of_week: None,
                        hour: Some(t.hour),
                        minute: Some(t.minute),
                        second: Some(t.second),
                        day_count: Some(t.day_count),
                        halted: Some(t.halted),
                        overflow: Some(t.overflow),
                    });
                } else {
                    println!("RTC (MBC3): 第{}天 {:02}:{:02}:{:02}{}{}",
                        t.day_count, t.hour, t.minute, t.second,
                        if t.halted { " [停止]" } else { "" },
                        if t.overflow { " [溢出]" } else { "" });
                }
                ExitCode::SUCCESS
            }
            None => {
                device::power_off(&mut link);
                op_err(json, "rtc", "RTC 读取失败");
                ExitCode::from(3)
            }
        }
    } else {
        match gba::ops::rtc::read_s3511(&mut link) {
            Some(t) => {
                device::power_off(&mut link);
                if json {
                    emit(&Event::RtcData {
                        ok: true,
                        kind: "gba".to_string(),
                        year: Some(t.year),
                        month: Some(t.month),
                        date: Some(t.date),
                        day_of_week: Some(t.day_of_week),
                        hour: Some(t.hour),
                        minute: Some(t.minute),
                        second: Some(t.second),
                        day_count: None, halted: None, overflow: None,
                    });
                } else {
                    println!("RTC (GBA/S3511): {:04}-{:02}-{:02} {:02}:{:02}:{:02} 星期{}",
                        t.year, t.month, t.date, t.hour, t.minute, t.second, t.day_of_week);
                }
                ExitCode::SUCCESS
            }
            None => {
                device::power_off(&mut link);
                op_err(json, "rtc", "RTC 读取失败（无 GPIO 功能？）");
                ExitCode::from(3)
            }
        }
    }
}

/// `cfb erase [--mbc]` —— 清空 ROM（整片擦除）。
pub fn cmd_erase(json: bool, port: Option<String>, mbc: bool) -> ExitCode {
    let Some(mut link) = open_powered(json, "erase", port, mbc) else {
        return ExitCode::from(3);
    };
    let ok = if mbc {
        mbc::ops::delete::erase_chip(&mut link, 240)
    } else {
        gba::ops::delete::erase_chip(&mut link, 240)
    };
    device::power_off(&mut link);
    finish(json, "erase", ok, 0, 0, 0.0)
}

/// `cfb dump --out <f> [--mbc] [--len N]` —— 导出 ROM 到文件。
pub fn cmd_dump(json: bool, port: Option<String>, out_path: &str, mbc: bool, len_opt: Option<u64>) -> ExitCode {
    let Some(mut link) = open_powered(json, "dump", port, mbc) else {
        return ExitCode::from(3);
    };
    // MBC：读卡带头识别代次（kind），缺省长度优先用头 0x148（游戏 ROM 大小），无效则回落 CFI。
    let (kind, len) = if mbc {
        let ct = mbc::ops::read::read_cart_byte(&mut link, 0x147).unwrap_or(0xFF);
        let k = mbc::data::MbcKind::from_cartridge_type(ct);
        let default_len = match mbc::ops::read::read_cart_byte(&mut link, 0x148) {
            Some(code) if code <= 8 => (32 * 1024u64) << code,
            _ => mbc::ops::read::rom_get_size(&mut link).0,
        };
        (k, len_opt.unwrap_or(default_len))
    } else {
        (
            mbc::data::MbcKind::Mbc5,
            len_opt.unwrap_or_else(|| gba::ops::read::read_info(&mut link).device_size),
        )
    };
    if len == 0 {
        device::power_off(&mut link);
        op_err(json, "dump", &i18n::t("op.no_size"));
        return ExitCode::from(3);
    }

    let mut last_mb = u64::MAX;
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let r = if mbc {
        mbc::ops::export::dump(&mut link, kind, len, out_path, &mut progress)
    } else {
        gba::ops::export::dump(&mut link, len, out_path, &mut progress)
    };
    device::power_off(&mut link);
    match r {
        Ok(()) => finish(json, "dump", true, len, 0, 0.0),
        Err(e) => {
            op_err(json, "dump", &i18n::tf("dump.fail", &[("err", &e.to_string())]));
            ExitCode::from(3)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个合法 GBA 头（含正确补码校验），无硬件验证解析/判别。
    fn synthetic_gba_header() -> [u8; 0xC0] {
        let mut h = [0u8; 0xC0];
        h[0xB2] = 0x96;
        h[0xA0..0xAC].copy_from_slice(b"POKEMON EMER");
        h[0xAC..0xB0].copy_from_slice(b"BPEE"); // 有 RTC
        h[0xBC] = 0x00;
        let sum: u32 = h[0xA0..=0xBC].iter().map(|&b| b as u32).sum();
        h[0xBD] = (0u32.wrapping_sub(0x19u32.wrapping_add(sum)) & 0xFF) as u8;
        h
    }

    #[test]
    fn detects_gba_and_parses_header() {
        let h = synthetic_gba_header();
        assert!(gba::ops::is_gba_header(&h));

        let g = gba::ops::parse_header(&h);
        assert_eq!(g.rom_title, "POKEMON EMER");
        assert_eq!(g.game_code, "BPEE");
        assert_eq!(g.revision, 0);
        assert!(g.rtc, "BPE 前缀应判为带 RTC");
        assert!(g.checksum.ok, "补码校验应通过");
        assert_eq!(g.game_name, "POKEMON EMER");
    }

    #[test]
    fn blank_is_not_gba() {
        let blank = [0xFFu8; 0xC0];
        assert!(common::ops::is_blank(&blank));
        assert!(!gba::ops::is_gba_header(&blank));
    }

    #[test]
    fn bad_checksum_is_not_gba() {
        let mut h = synthetic_gba_header();
        h[0xBD] ^= 0xFF;
        assert!(!gba::ops::is_gba_header(&h));
    }

    #[test]
    fn mbc_maptype_names() {
        assert_eq!(mbc::data::mbc_name(0x13), "MBC3");
        assert_eq!(mbc::data::mbc_name(0x1B), "MBC5");
        assert_eq!(mbc::data::mbc_name(0x00), "ROM ONLY");
    }

    #[test]
    fn parses_mbc3_header_and_checksum() {
        let mut rom = [0u8; 0x150];
        rom[0x134..0x13b].copy_from_slice(b"POKEMON");
        rom[0x143] = 0x80;
        rom[0x147] = 0x10;
        rom[0x148] = 0x05;
        let mut checksum = 0u8;
        for &byte in &rom[0x134..=0x14c] {
            checksum = checksum.wrapping_sub(byte).wrapping_sub(1);
        }
        rom[0x14d] = checksum;

        let h = mbc::ops::read::parse_header(&rom);
        assert_eq!(h.title, "POKEMON");
        assert_eq!(h.mbc_name, "MBC3");
        assert_eq!(h.rom_size_bytes, 1024 * 1024);
        assert!(h.rtc);
        assert!(h.header_checksum.ok);
    }
}
