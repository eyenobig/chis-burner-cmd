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
    // 与 burn/erase 共用 open_powered，避免 info 自管上电时序与成功路径不一致。
    // 接触抖动最多再整段重开 1 次。
    let mut last_err: Option<String> = None;
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(if mbc { 80 } else { 60 }));
        }
        let Some(mut link) = open_powered(json, "info", port.clone(), mbc) else {
            return ExitCode::from(3);
        };
        let port_name = link_port_name(&link);

        if mbc {
            // 头与 CFI 互相干扰：先 CFI 则头区常读全 FF；先大块读头则 CFI 偶发容量 0。
            // 策略：短探头 → CFI；有效头或 CFI 有容量即成功（空片靠 CFI，有游戏靠头）。
            let probe = mbc::ops::read::probe_live_header(&mut link);
            let (capacity, buffer) = mbc::ops::read::rom_get_size(&mut link);
            let raw = match probe {
                mbc::ops::read::HeaderProbe::Valid(h) => Some(h),
                mbc::ops::read::HeaderProbe::NoGame if capacity > 0 => Some([0xFFu8; 0x180]),
                mbc::ops::read::HeaderProbe::NoGame => None,
            };
            if let Some(raw) = raw {
                device::power_idle(&mut link);
                let _ = crate::config::save_selected(&port_name);
                if json {
                    emit_mbc_info(&port_name, capacity, buffer, &raw);
                } else {
                    let header = mbc::ops::read::parse_header(&raw);
                    print_mbc_human(
                        Some(&port_name),
                        capacity,
                        Some(buffer),
                        &header,
                        enrich_mbc(&raw, &header),
                    );
                }
                return ExitCode::SUCCESS;
            }
            device::power_idle(&mut link);
            last_err = Some(i18n::t("info.no_cartridge"));
            continue;
        }

        let flash = gba::ops::read_info(&mut link);
        let present = gba::ops::flash_present(&flash);
        if !present {
            device::power_idle(&mut link);
            last_err = Some(i18n::t("info.no_cartridge"));
            continue;
        }

        let header = {
            let mut h = [0u8; 0x180];
            if link.rom_read(0, &mut h) {
                Some(h)
            } else {
                None
            }
        };
        device::power_idle(&mut link);
        let _ = crate::config::save_selected(&port_name);

        let kind = match header.as_ref() {
            Some(h) if gba::ops::is_gba_header(h) => CartridgeKind::Gba,
            _ => CartridgeKind::Unknown,
        };
        let game: Option<GbaHeader> = match (kind, header.as_ref()) {
            (CartridgeKind::Gba, Some(h)) => Some(gba::ops::parse_header(h)),
            _ => None,
        };

        if json {
            emit_gba_info(
                &port_name,
                kind.as_str(),
                &flash.id_hex(),
                flash.device_size,
                flash.buffer_write_bytes,
                flash.sector_size,
                flash.sector_count,
                game.as_ref(),
                header.as_ref().map(|h| h.as_slice()),
            );
        } else {
            let friendly = header
                .as_ref()
                .and_then(|h| crate::gamedb::lookup_agb(h))
                .map(|e| e.gn);
            print_human(&port_name, kind, &flash, game.as_ref(), friendly.as_deref());
        }
        return ExitCode::SUCCESS;
    }

    emit_or_eprint(json, last_err.as_deref().unwrap_or(&i18n::t("info.no_cartridge")));
    ExitCode::from(3)
}

fn link_port_name(link: &CartridgeLink) -> String {
    link.port_name().to_string()
}

/// 发出一条 GBA 侧 `info` 事件（`cmd_info` 实时读 / `cmd_rom_info` 离线解析共用）。
/// `kind` 实时读时可能是 `"unknown"`（flash 在位但头部未识别）；离线解析恒 `"gba"`。
/// `game` 为 `None` 时游戏字段全部为 `null`。
/// `raw` 用于 header SHA1 查 `db_AGB`（友好名）；不足时回退 `game.rom_title`。
fn emit_gba_info(
    port: &str,
    kind: &str,
    id: &str,
    capacity_bytes: u64,
    buffer_write_bytes: u32,
    sector_size: u32,
    sector_count: u32,
    game: Option<&GbaHeader>,
    raw: Option<&[u8]>,
) {
    let db = raw.and_then(crate::gamedb::lookup_agb);
    let game_name = db
        .as_ref()
        .map(|e| e.gn.clone())
        .or_else(|| game.map(|g| g.game_name.clone()));
    emit(&Event::Info {
        port: port.to_string(),
        present: true,
        kind: kind.to_string(),
        id: id.to_string(),
        capacity_bytes,
        buffer_write_bytes,
        sector_size,
        sector_count,
        game_name,
        rom_title: game.map(|g| g.rom_title.clone()),
        game_code: game.map(|g| g.game_code.clone()),
        revision: game.map(|g| g.revision),
        rom_checksum: game.map(|g| g.checksum.clone()),
        rtc: game.map(|g| g.rtc),
        save_size_bytes: None, // GBA SRAM 大小需探测，info 阶段不读
        cartridge_type: None, // GBA 无此概念，游戏代号走 game_code
        mbc_name: None,
    });
}

/// db_DMG / 头解析合并后的友好字段（人类可读与 JSON 共用）。
struct MbcEnrich {
    game_name: String,
    game_code: Option<String>,
}

fn enrich_mbc(raw: &[u8], h: &MbcHeader) -> MbcEnrich {
    let db = crate::gamedb::lookup_dmg(raw);
    let game_name = db
        .as_ref()
        .map(|e| e.gn.clone())
        .unwrap_or_else(|| h.title.clone());
    // flashGBX：优先 db.gc（如 DMG-APAE）；否则用标题拆出的 4 字母代号。
    let game_code = db
        .and_then(|e| e.gc.filter(|s| !s.is_empty()))
        .or_else(|| h.game_code.clone());
    MbcEnrich { game_name, game_code }
}

/// 发出一条 `gb_mbc` 类 `info` 事件（`cmd_info` 的 live 读 / `cmd_rom_info` 离线解析共用）。
/// `capacity`/`buffer` 传 0 表示离线场景（无 flash CFI 数据），回落用头部 `rom_size_bytes`。
/// `raw` 须至少 0x150（查库最好 0x180）。
fn emit_mbc_info(port: &str, capacity: u64, buffer: u16, raw: &[u8]) {
    // 空片：只报 flash 在位，不把全 0xFF 头解析成 Unknown/rev 255。
    if raw.len() >= 0x150 && common::ops::is_blank(&raw[..0x150]) {
        emit(&Event::Info {
            port: port.to_string(),
            present: true,
            kind: "gb_mbc".to_string(),
            id: String::new(),
            capacity_bytes: capacity,
            buffer_write_bytes: buffer as u32,
            sector_size: 0,
            sector_count: 0,
            game_name: None,
            rom_title: None,
            game_code: None,
            revision: None,
            rom_checksum: None,
            rtc: None,
            save_size_bytes: None,
            cartridge_type: None,
            mbc_name: None,
        });
        return;
    }
    let h = mbc::ops::read::parse_header(raw);
    let en = enrich_mbc(raw, &h);
    emit(&Event::Info {
        port: port.to_string(),
        present: true,
        kind: "gb_mbc".to_string(),
        id: String::new(),
        capacity_bytes: if capacity == 0 { h.rom_size_bytes } else { capacity },
        buffer_write_bytes: buffer as u32,
        sector_size: 0,
        sector_count: 0,
        game_name: Some(en.game_name),
        rom_title: Some(h.title.clone()),
        game_code: en.game_code,
        revision: Some(h.revision),
        rom_checksum: Some(h.header_checksum.clone()),
        rtc: Some(h.rtc),
        save_size_bytes: if h.ram_size_bytes > 0 { Some(h.ram_size_bytes) } else { None },
        cartridge_type: Some(h.cartridge_type),
        mbc_name: Some(h.mbc_name.to_string()),
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

fn print_human(port: &str, kind: CartridgeKind, flash: &FlashInfo, game: Option<&GbaHeader>, friendly_name: Option<&str>) {
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
            let name = friendly_name.unwrap_or(&g.game_name);
            println!("{}", i18n::tf("info.game_name", &[("name", name)]));
            println!("{}", i18n::tf("info.rom_title", &[("title", &g.rom_title)]));
            println!("{}", i18n::tf("info.game_code", &[("code", &g.game_code)]));
            println!("{}", i18n::tf("info.revision", &[("rev", &g.revision.to_string())]));
            println!("{}", i18n::tf("info.rtc", &[("yn", &yn(g.rtc))]));
            print_checksum(&g.checksum);
        }
        None => println!("{}", i18n::t("info.no_game")),
    }
}

fn print_mbc_human(port: Option<&str>, capacity: u64, buffer: Option<u16>, h: &MbcHeader, en: MbcEnrich) {
    if let Some(port) = port {
        println!("{}", i18n::tf("info.port", &[("port", port)]));
        println!("{}", i18n::tf("info.cartridge", &[("kind", &i18n::t("kind.gb_mbc"))]));
    }
    println!("{}", i18n::tf("info.game_name", &[("name", &en.game_name)]));
    println!("{}", i18n::tf("info.rom_title", &[("title", &h.title)]));
    if let Some(code) = &en.game_code {
        println!("{}", i18n::tf("info.game_code", &[("code", code)]));
        println!("{}", i18n::tf("info.revision", &[("rev", &h.revision.to_string())]));
    }
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
            emit_gba_info("", "gba", "", bytes.len() as u64, 0, 0, 0, Some(&h), Some(&bytes));
        } else {
            let friendly = crate::gamedb::lookup_agb(&bytes).map(|e| e.gn);
            println!("{}", i18n::tf("rom_info.file", &[("path", path)]));
            println!("{}", i18n::tf("info.cartridge", &[("kind", &i18n::t("kind.gba"))]));
            println!("{}", i18n::tf("info.game_name", &[("name", friendly.as_deref().unwrap_or(&h.game_name))]));
            println!("{}", i18n::tf("info.rom_title", &[("title", &h.rom_title)]));
            println!("{}", i18n::tf("info.game_code", &[("code", &h.game_code)]));
            println!("{}", i18n::tf("info.revision", &[("rev", &h.revision.to_string())]));
            println!("{}", i18n::tf("info.capacity", &[("bytes", &bytes.len().to_string()), ("mb", &(bytes.len() / 1024 / 1024).to_string())]));
            println!("{}", i18n::tf("info.rtc", &[("yn", &yn(h.rtc))]));
            print_checksum(&h.checksum);
        }
    } else if bytes.len() >= 0x150 {
        let h = mbc::ops::read::parse_header(&bytes);
        let en = enrich_mbc(&bytes, &h);
        if json {
            emit_mbc_info("", 0, 0, &bytes);
        } else {
            println!("{}", i18n::tf("rom_info.file", &[("path", path)]));
            println!("{}", i18n::tf("info.cartridge", &[("kind", &i18n::t("kind.gb_mbc"))]));
            print_mbc_human(None, h.rom_size_bytes, None, &h, en);
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

/// 打开端口并按卡型上电。GBA **恒 3.3V**（无视 `voltage` 偏好）；MBC 读偏好否则默认 3.3V。
/// GBA 额外 `warm_up`。MBC 烧录路径内经 `soft_unplug_3v3`；命令结束回 `power_idle`(3.3V)。
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
        // 短 settle + GB 总线 warm_up；bank 初值走 switch_bank（与 burn/info 同一套锁存）。
        std::thread::sleep(std::time::Duration::from_millis(25));
        link.gbc_warm_up();
        mbc::ops::read::switch_bank(&mut link, 1, mbc::data::MbcKind::Mbc5);
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

/// 写入/擦除前的卡带在位判定（优化：无卡带直接中止，避免「空写 / 空擦」浪费时间
/// 且产生误导性的失败结果）。
///
/// 判定与 `cmd_info` 同源：
/// - GBA：读 flash ID + CFI，[`gba::ops::flash_present`]（有效 ID 或合理容量）。
/// - MBC：有效 GB 头，或 CFI 给出合理容量（**空白 flash 片也算在位**——可烧空白卡）。
///
/// 不在位时已 `power_idle` 并发错误事件 / 打印，返回 `false`，调用方据此早退（建议返回
/// `ExitCode::from(3)`，与「无烧录器 / 端口打不开」一致）。
fn ensure_cartridge_present(json: bool, cmd: &str, link: &mut CartridgeLink, mbc: bool) -> bool {
    let present = if mbc {
        match mbc::ops::read::probe_live_header(link) {
            mbc::ops::read::HeaderProbe::Valid(_) => true,
            // 空片 / 读不到头：靠 CFI 容量兜底判在位（空白卡可写）。
            mbc::ops::read::HeaderProbe::NoGame => mbc::ops::read::rom_get_size(link).0 > 0,
        }
    } else {
        gba::ops::flash_present(&gba::ops::read_info(link))
    };
    if !present {
        device::power_idle(link);
        op_err(json, cmd, &i18n::tf("op.no_cartridge", &[("cmd", cmd)]));
    }
    present
}

fn log_emit(json: bool, m: &str) {
    if json {
        emit(&Event::Log { message: m.to_string() });
    } else {
        println!("{m}");
    }
}

fn progress_emit(json: bool, done: u64, total: u64, last_tick: &mut u64) {
    if json {
        // 扇区擦除/整片擦除心跳 total 通常很小（扇区数 / 秒数）：每次都发。
        // 字节写入/校验/导出：按 total 自适应量化粒度，既避免逐包刷爆 UI，又保证
        // 进度均匀——MBC 的 256B 包和 GBA 的 4KiB 包都能在合理间隔内各发一次。
        let sector_like = total > 0 && total < 4096;
        let tick = if sector_like { done } else { done / tick_step(total) };
        if sector_like || done == 0 || done >= total || tick != *last_tick {
            *last_tick = tick;
            emit(&Event::Progress { done, total });
        }
    } else {
        let mb = done / (1 << 20);
        if mb != *last_tick {
            *last_tick = mb;
            println!("  {} / {} MB", mb, total / (1 << 20));
        }
    }
}

/// 字节进度量化步长：按 total 分档，大 ROM 步长大（控事件数），小 ROM 步长小（保均匀）。
/// - 256B 包（MBC）：2KiB 步长 → 每 8 包发一次；4KiB 包（GBA）：2KiB 步长 → 每包都发。
/// - 大 ROM（≥1MB）：16KiB 步长，4MB 全程约 256 个事件，不刷爆 stdout/UI。
fn tick_step(total: u64) -> u64 {
    const KIB: u64 = 1024;
    if total >= 1024 * KIB {
        16 * KIB
    } else if total >= 256 * KIB {
        8 * KIB
    } else {
        2 * KIB
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

/// 从 ROM 文件头判别平台：GBA（0xB2==0x96 + 头校验）/ GB·GBC（GB 头校验）/ 无法判定。
/// 与 `cmd_rom_info` 的判别同源。头不全或校验不过时返回 [`CartridgeKind::Unknown`]，
/// 调用方据此只对「确定的不匹配」拦截，避免误伤小文件 / 空片。
fn detect_rom_platform(bytes: &[u8]) -> CartridgeKind {
    if bytes.len() >= 0xC0 && gba::ops::is_gba_header(bytes) {
        CartridgeKind::Gba
    } else if bytes.len() >= 0x150 && mbc::ops::read::is_gb_header(bytes) {
        CartridgeKind::GbMbc
    } else {
        CartridgeKind::Unknown
    }
}

/// 卡带/ROM 平台的本地化名称（"GBA" / "GB/GBC" / "未识别/空片"）。
fn kind_label(kind: CartridgeKind) -> String {
    i18n::t(match kind {
        CartridgeKind::Gba => "kind.gba",
        CartridgeKind::GbMbc => "kind.gb_mbc",
        CartridgeKind::Unknown => "kind.unknown",
    })
}

/// 数字 → MbcKind（main.rs 的 --mbc-kind 解析用）。
pub fn mbc_kind(n: u8) -> mbc::data::MbcKind {
    match n {
        1 => mbc::data::MbcKind::Mbc1,
        2 => mbc::data::MbcKind::Mbc2,
        3 => mbc::data::MbcKind::Mbc3,
        _ => mbc::data::MbcKind::Mbc5,
    }
}

/// `cfb burn --rom <f> [--mbc] [--no-erase]` —— 写入 ROM。
pub fn cmd_burn(
    json: bool,
    port: Option<String>,
    rom_path: &str,
    mbc: bool,
    chip_erase: bool,
    unlock_ppb: bool,
    verify: bool,
    no_erase: bool,
    mbc_kind: Option<mbc::data::MbcKind>,
) -> ExitCode {
    let data = match std::fs::read(rom_path) {
        Ok(d) => d,
        Err(e) => {
            op_err(json, "burn", &i18n::tf("op.read_fail", &[("path", rom_path), ("err", &e.to_string())]));
            return ExitCode::from(2);
        }
    };

    // ROM 平台须与目标卡带一致：GBA ROM 只能烧到 GBA 卡，GB/GBC ROM 只能烧到 GB/GBC 卡。
    // 头无法判定（小文件 / 空片）时不拦截，只对「确定的不匹配」报错，避免误伤。
    let rom_kind = detect_rom_platform(&data);
    let target_kind = if mbc { CartridgeKind::GbMbc } else { CartridgeKind::Gba };
    if rom_kind != CartridgeKind::Unknown && rom_kind != target_kind {
        let rom_label = kind_label(rom_kind);
        let cart_label = kind_label(target_kind);
        op_err(
            json,
            "burn",
            &i18n::tf(
                "op.rom_platform_mismatch",
                &[("rom", rom_label.as_str()), ("cart", cart_label.as_str())],
            ),
        );
        return ExitCode::from(2);
    }

    let Some(mut link) = open_powered(json, "burn", port, mbc) else {
        return ExitCode::from(3);
    };

    // 无卡带直接中止，避免空写（GBA 读 flash ID/CFI；MBC 探头 + CFI 容量兜底）。
    if !ensure_cartridge_present(json, "burn", &mut link, mbc) {
        return ExitCode::from(3);
    }

    // GBA：对齐 beggar_socket，每次 mission 软件插拔清 MCU 残留后再烧。
    if !mbc {
        if let Err(e) = link.soft_unplug_gba() {
            op_err(json, "burn", &format!("GBA 软件插拔失败: {e}"));
            return ExitCode::from(3);
        }
    }

    let mut last_mb = u64::MAX;
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let mut log = |m: &str| log_emit(json, m);

    let res = if mbc {
        // MBC：默认路径已是整片+扇区；`chip_erase` 标志与 GBA/CLI 对齐（MBC 侧忽略）
        mbc::ops::write::burn(&mut link, &data, verify, chip_erase, no_erase, mbc_kind, &mut progress, &mut log)
    } else {
        // 两线统一：默认 false=只擦 ROM 范围；--chip-erase=true 整片清场
        let opt = BurnOptions { chip_erase, unlock_ppb, verify, no_erase };
        gba::ops::write::burn(&mut link, &data, &opt, &mut progress, &mut log)
    };
    device::power_idle(&mut link);
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
                device::power_idle(&mut link);
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
                device::power_idle(&mut link);
                op_err(json, "rtc", "RTC 读取失败");
                ExitCode::from(3)
            }
        }
    } else {
        match gba::ops::rtc::read_s3511(&mut link) {
            Some(t) => {
                device::power_idle(&mut link);
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
                device::power_idle(&mut link);
                op_err(json, "rtc", "RTC 读取失败（无 GPIO 功能？）");
                ExitCode::from(3)
            }
        }
    }
}

/// `cfb erase [--mbc]` —— 清空 ROM（按 CFI 扇区逐个擦除，带进度；容量未知时回落整片擦除）。
pub fn cmd_erase(
    json: bool,
    port: Option<String>,
    mbc: bool,
    mbc_kind: Option<mbc::data::MbcKind>,
    boot: bool,
) -> ExitCode {
    let Some(mut link) = open_powered(json, "erase", port, mbc) else {
        return ExitCode::from(3);
    };
    // 无卡带直接中止，避免空擦。
    if !ensure_cartridge_present(json, "erase", &mut link, mbc) {
        return ExitCode::from(3);
    }
    let t0 = std::time::Instant::now();
    let mut last_mb = 0u64;
    let mut log = |m: &str| log_emit(json, m);
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let ok = if mbc {
        // CFI 查容量+扇区大小；卡上实读 MBC 代次决定 bank/地址映射（同 burn 逻辑）。
        // CFI 无容量时与 info 一致：回落头 0x148，再不行默认 8MiB；仍走扇区擦除以便上报进度。
        let (cfi_size, _buf, cfi_sector) = mbc::ops::read::rom_get_cfi(&mut link);
        let id = mbc::ops::read::rom_get_id(&mut link);
        // 规则库匹配（同 burn）：profile 显式扇区优先，擦除命令走 profile 序列
        let prof = {
            let all = crate::profile::load_all();
            let id8 = [id[0], id[1], id[2], id[3], 0, 0, 0, 0];
            crate::profile::match_by_id(&all, &id8).map(|p| p.clone())
        };
        if let Some(p) = &prof {
            log(&format!("Profile: {}", p.name));
        }
        let sector_size = prof
            .as_ref()
            .and_then(crate::profile::uniform_sector_size)
            .unwrap_or_else(|| mbc::ops::read::effective_erase_sector(&id, cfi_sector));
        // 总线默认 MBC5（同 burn）；--mbc-kind 手动兜底
        let kind = mbc_kind.unwrap_or(mbc::data::MbcKind::Mbc5);
        let device_size = if cfi_size > 0 {
            cfi_size
        } else {
            let fb = match mbc::ops::read::read_cart_byte(&mut link, 0x148) {
                Some(code) if code <= 8 => (32 * 1024u64) << code,
                _ => 8 * 1024 * 1024,
            };
            log(&format!(
                "CFI 无容量，按 {fb} 扇区擦除 (sector={sector_size})"
            ));
            fb
        };
        link.gbc_write(0x00, &[0xf0]); // CFI 后强制复位，避免残留查询模式导致擦除无响应
        link.gbc_warm_up();
        log(&format!("擦除全片: flash={device_size} sector={sector_size}"));
        let sector_ok = mbc::ops::delete::erase_range_logged(
            &mut link,
            kind,
            0,
            device_size,
            sector_size,
            prof.as_ref(),
            &mut progress,
            &mut log,
        );
        let ok = if sector_ok {
            true
        } else {
            log("扇区擦除失败，回落整片擦除...");
            progress(0, 1);
            mbc::ops::delete::erase_chip_logged(&mut link, 90, &mut progress, &mut log)
        };
        // --boot：MBC5 线性映射下主擦除从 phys 0x4000 起，物理 0x0-0x3FFF 的
        // 开机窗/隐藏头部区不会被覆盖——旧头部残留会让识别仍报旧游戏。
        // 显式要求时单独擦掉（该窗走 0x30@0x0000 专用序列），让卡真正干净。
        if ok && boot {
            log("擦除开机窗（隐藏头部区）...");
            if mbc::ops::write::erase_boot_window(&mut link, &mut log) {
                log("开机窗擦除完成");
            } else {
                log("开机窗擦除失败（主区已清空）");
            }
        }
        ok
    } else {
        let flash = gba::ops::read::read_info(&mut link);
        let (device_size, sector_size) = if flash.device_size > 0 {
            (flash.device_size, flash.sector_size)
        } else {
            // CFI 失败：默认 16MiB / 64KiB（常见 GBA 复制卡），保证仍有进度事件
            let fb = 16 * 1024 * 1024u64;
            let ss = if flash.sector_size > 0 {
                flash.sector_size
            } else {
                64 * 1024
            };
            log(&format!("CFI 无容量，按 {fb} 扇区擦除 (sector={ss})"));
            (fb, ss)
        };
        log(&format!(
            "擦除全片: flash={device_size} sector={sector_size}"
        ));
        let sector_ok = gba::ops::delete::erase_range_logged(
            &mut link,
            0,
            device_size as u32,
            sector_size,
            &mut progress,
            &mut log,
        );
        if sector_ok {
            true
        } else {
            log("扇区擦除失败，回落整片擦除...");
            let chip_ok =
                gba::ops::delete::erase_chip_logged(&mut link, 240, &mut progress, &mut log);
            chip_ok
        }
    };
    let secs = t0.elapsed().as_secs_f64();
    device::power_idle(&mut link);
    if json {
        log_emit(
            true,
            &format!(
                "{} · {:.1}s",
                if ok { "擦除完成" } else { "擦除失败" },
                secs
            ),
        );
    }
    finish(json, "erase", ok, 0, 0, secs)
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
        device::power_idle(&mut link);
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
    device::power_idle(&mut link);
    match r {
        Ok(()) => finish(json, "dump", true, len, 0, 0.0),
        Err(e) => {
            op_err(json, "dump", &i18n::tf("dump.fail", &[("err", &e.to_string())]));
            ExitCode::from(3)
        }
    }
}

// ==================== 存档 (save) 命令 ====================

/// 解析 `--type`（默认 SRAM）；非法返回 None（由调用方报错）。
fn parse_save_type(json: bool, cmd: &str, raw: Option<String>) -> Option<gba::data::SaveType> {
    match raw.as_deref() {
        None => Some(gba::data::SaveType::Sram),
        Some(s) => match gba::data::SaveType::from_user(s) {
            Some(st) => Some(st),
            None => {
                op_err(json, cmd, &i18n::tf("save.type_invalid", &[("v", s)]));
                None
            }
        },
    }
}

/// MBC 不支持 FLASH/EEPROM。
fn mbc_save_type(st: gba::data::SaveType) -> Result<gba::data::SaveType, ()> {
    match st {
        gba::data::SaveType::Flash
        | gba::data::SaveType::Eeprom4k
        | gba::data::SaveType::Eeprom64k => Err(()),
        other => Ok(other),
    }
}

/// 读卡带确定 MBC 代次 + 默认存档大小（头 0x149）。
fn mbc_save_defaults(link: &mut CartridgeLink) -> (mbc::data::MbcKind, u64) {
    if let Some(raw) = mbc::ops::read::read_live_header(link) {
        let header = mbc::ops::read::parse_header(&raw);
        let kind = mbc::data::MbcKind::from_cartridge_type(header.cartridge_type);
        return (kind, header.ram_size_bytes);
    }
    let ct = mbc::ops::read::read_cart_byte(link, 0x147).unwrap_or(0xFF);
    let kind = mbc::data::MbcKind::from_cartridge_type(ct);
    let ram = if kind == mbc::data::MbcKind::Mbc2 {
        512
    } else {
        match mbc::ops::read::read_cart_byte(link, 0x149) {
            Some(code) => mbc::ops::read::ram_size(code),
            None => 0,
        }
    };
    (kind, ram)
}

/// `cfb save-dump --out <f> [--mbc] [--type ...] [--len N]` —— 导出存档。
pub fn cmd_save_dump(
    json: bool,
    port: Option<String>,
    out_path: &str,
    mbc: bool,
    type_raw: Option<String>,
    len_opt: Option<u64>,
) -> ExitCode {
    let cmd = "save-dump";
    let Some(st) = parse_save_type(json, cmd, type_raw) else {
        return ExitCode::from(2);
    };
    let Some(mut link) = open_powered(json, cmd, port, mbc) else {
        return ExitCode::from(3);
    };
    let mut last_mb = u64::MAX;
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let mut log = |m: &str| log_emit(json, m);

    let res = if mbc {
        let st = match mbc_save_type(st) {
            Ok(s) => s,
            Err(()) => {
                device::power_idle(&mut link);
                op_err(json, cmd, "MBC 不支持 FLASH 存档类型（用 sram / fram）");
                return ExitCode::from(2);
            }
        };
        {
            let (kind, ram) = mbc_save_defaults(&mut link);
            let len = len_opt.unwrap_or(ram);
            if len == 0 {
                device::power_idle(&mut link);
                op_err(json, cmd, &i18n::t("op.no_size"));
                return ExitCode::from(3);
            }
            let r = mbc::ops::save::dump(&mut link, kind, matches!(st, gba::data::SaveType::Fram), len, out_path, &mut log, &mut progress);
            emit_save_info(json, st, None, r.bytes);
            r
        }
    } else {
        match st {
            gba::data::SaveType::Eeprom4k | gba::data::SaveType::Eeprom64k => {
                let expected = st.eeprom_size().unwrap();
                if len_opt.map(|len| len != expected).unwrap_or(false) {
                    device::power_idle(&mut link);
                    op_err(json, cmd, &format!("{} 存档长度固定为 {} 字节", st.label(), expected));
                    return ExitCode::from(2);
                }
                let r = gba::ops::save::dump_eeprom(&mut link, st, out_path, &mut log, &mut progress);
                emit_save_info(json, st, None, r.bytes);
                r
            }
            gba::data::SaveType::Sram | gba::data::SaveType::Flash | gba::data::SaveType::Fram => {
                // GBA SRAM/FLASH/FRAM 默认 64KiB（与 C# 默认一致），可用 --len 覆盖。
                let len = len_opt.unwrap_or(64 * 1024);
                let r = gba::ops::save::dump(&mut link, st, len, out_path, &mut log, &mut progress);
                emit_save_info(json, st, None, r.bytes);
                r
            }
        }
    };
    device::power_idle(&mut link);
    finish(json, cmd, res.success, res.bytes, res.mismatch_bytes, res.seconds)
}

/// `cfb save-write --file <f> [--mbc] [--type ...]` —— 写入存档。
pub fn cmd_save_write(
    json: bool,
    port: Option<String>,
    file_path: &str,
    mbc: bool,
    type_raw: Option<String>,
) -> ExitCode {
    let cmd = "save-write";
    let Some(st) = parse_save_type(json, cmd, type_raw) else {
        return ExitCode::from(2);
    };
    let data = match std::fs::read(file_path) {
        Ok(d) => d,
        Err(e) => {
            op_err(json, cmd, &i18n::tf("op.read_fail", &[("path", file_path), ("err", &e.to_string())]));
            return ExitCode::from(2);
        }
    };
    let Some(mut link) = open_powered(json, cmd, port, mbc) else {
        return ExitCode::from(3);
    };
    // 无卡带直接中止，避免空写存档。
    if !ensure_cartridge_present(json, cmd, &mut link, mbc) {
        return ExitCode::from(3);
    }
    let mut last_mb = u64::MAX;
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let mut log = |m: &str| log_emit(json, m);

    let res = if mbc {
        let st = match mbc_save_type(st) {
            Ok(s) => s,
            Err(()) => {
                device::power_idle(&mut link);
                op_err(json, cmd, "MBC 不支持 FLASH 存档类型（用 sram / fram）");
                return ExitCode::from(2);
            }
        };
        {
            let (kind, _) = mbc_save_defaults(&mut link);
            let r = mbc::ops::save::write(&mut link, kind, matches!(st, gba::data::SaveType::Fram), &data, &mut log, &mut progress);
            emit_save_info(json, st, None, r.bytes);
            r
        }
    } else {
        match st {
            gba::data::SaveType::Eeprom4k | gba::data::SaveType::Eeprom64k => {
                let r = gba::ops::save::write_eeprom(&mut link, st, &data, &mut log, &mut progress);
                emit_save_info(json, st, None, r.bytes);
                r
            }
            gba::data::SaveType::Sram | gba::data::SaveType::Flash | gba::data::SaveType::Fram => {
                let r = gba::ops::save::write(&mut link, st, &data, &mut log, &mut progress);
                emit_save_info(json, st, None, r.bytes);
                r
            }
        }
    };
    device::power_idle(&mut link);
    finish(json, cmd, res.success, res.bytes, res.mismatch_bytes, res.seconds)
}

/// `cfb save-verify --file <f> [--mbc] [--type ...]` —— 校验存档。
pub fn cmd_save_verify(
    json: bool,
    port: Option<String>,
    file_path: &str,
    mbc: bool,
    type_raw: Option<String>,
) -> ExitCode {
    let cmd = "save-verify";
    let Some(st) = parse_save_type(json, cmd, type_raw) else {
        return ExitCode::from(2);
    };
    let data = match std::fs::read(file_path) {
        Ok(d) => d,
        Err(e) => {
            op_err(json, cmd, &i18n::tf("op.read_fail", &[("path", file_path), ("err", &e.to_string())]));
            return ExitCode::from(2);
        }
    };
    let Some(mut link) = open_powered(json, cmd, port, mbc) else {
        return ExitCode::from(3);
    };
    let mut last_mb = u64::MAX;
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let mut log = |m: &str| log_emit(json, m);

    let res = if mbc {
        let st = match mbc_save_type(st) {
            Ok(s) => s,
            Err(()) => {
                device::power_idle(&mut link);
                op_err(json, cmd, "MBC 不支持 FLASH 存档类型（用 sram / fram）");
                return ExitCode::from(2);
            }
        };
        {
            let (kind, _) = mbc_save_defaults(&mut link);
            let r = mbc::ops::save::verify(&mut link, kind, matches!(st, gba::data::SaveType::Fram), &data, &mut log, &mut progress);
            emit_save_info(json, st, None, r.bytes);
            r
        }
    } else {
        match st {
            gba::data::SaveType::Eeprom4k | gba::data::SaveType::Eeprom64k => {
                let r = gba::ops::save::verify_eeprom(&mut link, st, &data, &mut log, &mut progress);
                emit_save_info(json, st, None, r.bytes);
                r
            }
            gba::data::SaveType::Sram | gba::data::SaveType::Flash | gba::data::SaveType::Fram => {
                let r = gba::ops::save::verify(&mut link, st, &data, &mut log, &mut progress);
                emit_save_info(json, st, None, r.bytes);
                r
            }
        }
    };
    device::power_idle(&mut link);
    finish(json, cmd, res.success, res.bytes, res.mismatch_bytes, res.seconds)
}

/// `cfb save-erase [--mbc] [--type ...] [--len N]` —— 擦除存档（填 0xFF；FLASH 写前整片擦除）。
pub fn cmd_save_erase(
    json: bool,
    port: Option<String>,
    mbc: bool,
    type_raw: Option<String>,
    len_opt: Option<u64>,
) -> ExitCode {
    let cmd = "save-erase";
    let Some(st) = parse_save_type(json, cmd, type_raw) else {
        return ExitCode::from(2);
    };
    let Some(mut link) = open_powered(json, cmd, port, mbc) else {
        return ExitCode::from(3);
    };
    // 无卡带直接中止，避免空擦存档（save-erase 即写入全 0xFF）。
    if !ensure_cartridge_present(json, cmd, &mut link, mbc) {
        return ExitCode::from(3);
    }
    let mut last_mb = u64::MAX;
    let mut progress = |d: u64, t: u64| progress_emit(json, d, t, &mut last_mb);
    let mut log = |m: &str| log_emit(json, m);

    let res = if mbc {
        let st = match mbc_save_type(st) {
            Ok(s) => s,
            Err(()) => {
                device::power_idle(&mut link);
                op_err(json, cmd, "MBC 不支持 FLASH 存档类型（用 sram / fram）");
                return ExitCode::from(2);
            }
        };
        {
            let (kind, ram) = mbc_save_defaults(&mut link);
            let len = len_opt.unwrap_or(ram);
            if len == 0 {
                device::power_idle(&mut link);
                op_err(json, cmd, &i18n::t("op.no_size"));
                return ExitCode::from(3);
            }
            log(&i18n::t("save.erase"));
            let data = vec![0xffu8; len as usize];
            let r = mbc::ops::save::write(
                &mut link,
                kind,
                matches!(st, gba::data::SaveType::Fram),
                &data,
                &mut log,
                &mut progress,
            );
            emit_save_info(json, st, None, r.bytes);
            r
        }
    } else {
        match st {
            gba::data::SaveType::Eeprom4k | gba::data::SaveType::Eeprom64k => {
                let expected = st.eeprom_size().unwrap();
                if len_opt.map(|len| len != expected).unwrap_or(false) {
                    device::power_idle(&mut link);
                    op_err(json, cmd, &format!("{} 存档长度固定为 {} 字节", st.label(), expected));
                    return ExitCode::from(2);
                }
                log(&i18n::t("save.erase"));
                let data = vec![0xffu8; expected as usize];
                let r = gba::ops::save::write_eeprom(&mut link, st, &data, &mut log, &mut progress);
                emit_save_info(json, st, None, r.bytes);
                r
            }
            gba::data::SaveType::Sram | gba::data::SaveType::Flash | gba::data::SaveType::Fram => {
                let len = len_opt.unwrap_or(64 * 1024);
                log(&i18n::t("save.erase"));
                let data = vec![0xffu8; len as usize];
                let r = gba::ops::save::write(&mut link, st, &data, &mut log, &mut progress);
                emit_save_info(json, st, None, r.bytes);
                r
            }
        }
    };
    device::power_idle(&mut link);
    finish(json, cmd, res.success, res.bytes, res.mismatch_bytes, res.seconds)
}

/// 发出 save_info 事件（人类模式静默；offset 现恒为 None，保留参数兼容客户端契约）。
fn emit_save_info(json: bool, st: gba::data::SaveType, offset: Option<u64>, size: u64) {
    if json {
        emit(&Event::SaveInfo {
            save_type: st.label().to_string(),
            offset,
            size,
        });
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
        rom[0x149] = 0x03; // 32KB RAM
        let mut checksum = 0u8;
        for &byte in &rom[0x134..=0x14c] {
            checksum = checksum.wrapping_sub(byte).wrapping_sub(1);
        }
        rom[0x14d] = checksum;

        assert!(mbc::ops::read::is_gb_header(&rom));
        let h = mbc::ops::read::parse_header(&rom);
        assert_eq!(h.title, "POKEMON");
        assert_eq!(h.mbc_name, "MBC3");
        assert_eq!(h.rom_size_bytes, 1024 * 1024);
        assert_eq!(h.ram_size_bytes, 32 * 1024, "0x149=0x03 应为 32KB RAM");
        assert!(h.rtc);
        assert!(h.header_checksum.ok);
    }

    #[test]
    fn blank_is_not_gb_header() {
        let blank = [0xFFu8; 0x150];
        assert!(!mbc::ops::read::is_gb_header(&blank));
    }

    #[test]
    fn mbc_ram_size_table() {
        // 标准 GB RAM size 编码（头 0x149）。
        assert_eq!(mbc::ops::read::ram_size(0x00), 0);
        assert_eq!(mbc::ops::read::ram_size(0x01), 2 * 1024);
        assert_eq!(mbc::ops::read::ram_size(0x02), 8 * 1024);
        assert_eq!(mbc::ops::read::ram_size(0x03), 32 * 1024);
        assert_eq!(mbc::ops::read::ram_size(0x04), 128 * 1024);
        assert_eq!(mbc::ops::read::ram_size(0x05), 64 * 1024);
        assert_eq!(mbc::ops::read::ram_size(0xFF), 0, "未知编码按 0");
    }

    #[test]
    fn save_type_parses() {
        use gba::data::SaveType;
        assert_eq!(SaveType::from_user("eeprom4k"), Some(SaveType::Eeprom4k));
        assert_eq!(SaveType::from_user("EEPROM64K"), Some(SaveType::Eeprom64k));
        assert_eq!(SaveType::from_user("eeprom512b"), Some(SaveType::Eeprom4k));
        assert_eq!(SaveType::from_user("sram"), Some(SaveType::Sram));
        assert_eq!(SaveType::from_user("FLASH"), Some(SaveType::Flash));
        assert_eq!(SaveType::from_user("Fram"), Some(SaveType::Fram));
        assert_eq!(SaveType::from_user("batteryless"), None);
        assert_eq!(SaveType::from_user("bat"), None);
        assert_eq!(SaveType::from_user("nope"), None);
        assert_eq!(SaveType::Sram.label(), "SRAM");
        assert_eq!(SaveType::Eeprom4k.eeprom_size(), Some(512));
        assert_eq!(SaveType::Eeprom64k.eeprom_size(), Some(8192));
    }

    #[test]
    fn detect_rom_platform_gba_vs_gb() {
        // GBA 头 → GBA（0xC0 < 0x150，不会同时过 GB 头校验）。
        let gba = synthetic_gba_header();
        assert_eq!(detect_rom_platform(&gba), CartridgeKind::Gba);

        // GB/GBC 头 → GbMbc，且不应被误判成 GBA。
        let mut gb = [0u8; 0x150];
        gb[0x134..0x13b].copy_from_slice(b"POKEMON");
        gb[0x143] = 0x80; // CGB
        gb[0x147] = 0x10; // MBC3+TIMER
        gb[0x148] = 0x05;
        gb[0x149] = 0x03;
        let mut checksum = 0u8;
        for &byte in &gb[0x134..=0x14c] {
            checksum = checksum.wrapping_sub(byte).wrapping_sub(1);
        }
        gb[0x14d] = checksum;
        assert_eq!(detect_rom_platform(&gb), CartridgeKind::GbMbc);
        assert!(!gba::ops::is_gba_header(&gb));

        // 太小 / 全 FF → 无法判定（不拦截，避免误伤小文件 / 空片）。
        assert_eq!(detect_rom_platform(&[0u8; 0x80]), CartridgeKind::Unknown);
        assert_eq!(detect_rom_platform(&[0xFFu8; 0x200]), CartridgeKind::Unknown);
    }
}
