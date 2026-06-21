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

/// `cfb info` —— 读 flash 芯片 + 卡带/游戏信息（人类可读 / `--json`）。
pub fn cmd_info(json: bool, port: Option<String>) -> ExitCode {
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

    device::power(&mut link, device::Voltage::V3_3); // GBA = 3.3V
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
        emit(&Event::Info {
            port: port.clone(),
            present,
            kind: kind.as_str().to_string(),
            id: flash.id_hex(),
            capacity_bytes: flash.device_size,
            buffer_write_bytes: flash.buffer_write_bytes,
            sector_size: flash.sector_size,
            sector_count: flash.sector_count,
            game_name: game.as_ref().map(|g| g.game_name.clone()),
            rom_title: game.as_ref().map(|g| g.rom_title.clone()),
            game_code: game.as_ref().map(|g| g.game_code.clone()),
            revision: game.as_ref().map(|g| g.revision),
            rom_checksum: game.as_ref().map(|g| g.checksum.clone()),
            rtc: game.as_ref().map(|g| g.rtc),
        });
    } else {
        print_human(&port, kind, &flash, game.as_ref());
    }
    ExitCode::SUCCESS
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
            let c = &g.checksum;
            if c.ok {
                println!("{}", i18n::tf("info.checksum_ok", &[("stored", &format!("{:02X}", c.stored))]));
            } else {
                println!(
                    "{}",
                    i18n::tf(
                        "info.checksum_bad",
                        &[("stored", &format!("{:02X}", c.stored)), ("computed", &format!("{:02X}", c.computed))]
                    )
                );
            }
        }
        None => println!("{}", i18n::t("info.no_game")),
    }
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
        mbc::ops::write::burn(&mut link, &data, &mut progress, &mut log)
    } else {
        let opt = BurnOptions { chip_erase, unlock_ppb, verify };
        gba::ops::write::burn(&mut link, &data, &opt, &mut progress, &mut log)
    };
    device::power_off(&mut link);
    finish(json, "burn", res.success, res.bytes_written, res.mismatch_bytes, res.seconds)
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
    let len = match len_opt {
        Some(l) => l,
        None => {
            if mbc {
                mbc::ops::read::rom_get_size(&mut link).0
            } else {
                gba::ops::read::read_info(&mut link).device_size
            }
        }
    };
    if len == 0 {
        device::power_off(&mut link);
        op_err(json, "dump", &i18n::t("op.no_size"));
        return ExitCode::from(3);
    }

    let mut last_mb = u64::MAX;
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let r = if mbc {
        mbc::ops::export::dump(&mut link, len, out_path, &mut progress)
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
}
