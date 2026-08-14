//! GBA · 写：编程 / 烧录 ROM。
//!
//! 对齐 beggar_socket WinForms（`mission_eraseChip` + `mission_programRom`）稳定路径：
//! 默认整片擦后连续写；编程失败仅 DTR/RTS 复位重试（见 `CartridgeLink::rom_program`），
//! 不做 Core 式频繁关口 reconnect。保留 FlashGBX profile、PPB、校验修复。

use std::time::Instant;

use super::delete::{
    chip_erase_profile_logged, erase_chip_logged, erase_sector, sector_erase_profile,
    unlock_all_ppb_logged,
};
use super::read::read_info;
use crate::cartridge_link::CartridgeLink;
use crate::profile;
use crate::progress_display::{Phase, ProgressLog};
use crate::rom::gba::data::{BurnOptions, BurnResult, SECTOR};

const PACKET: usize = 4096;
const SECTOR_U64: u64 = SECTOR as u64;
/// 整片擦除兜底超时（秒）；对齐临时成功路径 / WinForms。
const CHIP_ERASE_TIMEOUT_SECS: u64 = 240;

/// 连续编程 [from,to)。`rom_program` 内已 4× DTR；包失败即停。返回首个失败地址或 None。
fn program_flow(
    link: &mut CartridgeLink,
    rom: &[u8],
    from: u64,
    to: u64,
    buf_wr: u16,
    res: &mut BurnResult,
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Option<u64> {
    let mut pos = from;
    // 包级原地重试：新批次烧录器持续写入会偶发掉包（rom_program 内部 4 次 MCU 复位
    // 重试仍不够，2026-08-15 实测 4-16KB 处挂）；这里失败后 0xF0+复位再重发同一包，
    // 同包连续 5 次不过才真正放弃（原地续传，不整段重来）。
    let mut fail_streak = 0u32;
    while pos < to {
        let len = ((to - pos) as usize).min(PACKET);
        let pk = &rom[pos as usize..pos as usize + len];

        if !link.rom_program(pos as u32, pk, buf_wr) {
            fail_streak += 1;
            if fail_streak >= 5 {
                return Some(pos);
            }
            link.reset_mcu_buffer();
            link.rom_write(0, &[0xf0, 0x00]);
            std::thread::sleep(std::time::Duration::from_millis(30));
            continue;
        }
        fail_streak = 0;

        pos += len as u64;
        res.bytes_written += len as u64;
        progress(pos, total);
    }
    None
}

/// 逐扇区校验，返回不一致的扇区基址集合 + 总不符字节数。
fn find_bad_sectors(link: &mut CartridgeLink, rom: &[u8], total: u64) -> (Vec<u64>, u32) {
    use std::collections::BTreeSet;
    let mut bad = BTreeSet::new();
    let mut mismatch = 0u32;
    let mut pos = 0u64;
    let mut buf = vec![0u8; PACKET];
    while pos < total {
        let len = ((total - pos) as usize).min(PACKET);
        let b = &mut buf[..len];
        if !link.rom_read(pos as u32, b) {
            link.reset_mcu_buffer();
            continue;
        }
        for i in 0..len {
            if b[i] != rom[pos as usize + i] {
                bad.insert(((pos + i as u64) / SECTOR_U64) * SECTOR_U64);
                mismatch += 1;
            }
        }
        pos += len as u64;
    }
    (bad.into_iter().collect(), mismatch)
}

/// 完整烧录：(可选解锁PPB) → (整片或逐扇区)擦除+编程 → 校验+修复。
///
/// 默认 `chip_erase=false`（逐扇区只擦 ROM 范围，快路径）；`--chip-erase` 整片清场。
pub fn burn(
    link: &mut CartridgeLink,
    rom: &[u8],
    opt: &BurnOptions,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
) -> BurnResult {
    let length = rom.len() as u64;
    let mut res = BurnResult {
        success: false,
        bytes_written: 0,
        reconnects: 0,
        first_bad: None,
        mismatch_bytes: 0,
        seconds: 0.0,
    };
    let start = Instant::now();

    // 常见 GBA NOR 厂商标识；总线乱读时 ID 会像 `00 03 42…`，不应当有效芯片。
    let id_sane = |id: &[u8; 8]| {
        matches!(id[0], 0x01 | 0xC2 | 0x20 | 0x89 | 0x2C | 0xBF | 0x1C | 0x37 | 0xEC)
            && !id.iter().all(|&b| b == 0x00 || b == 0xFF)
    };

    let mut buf_wr: u16 = 32; // S29GL256 默认；CFI 可覆盖
    let mut info = read_info(link);
    if info.id.as_ref().is_none_or(|id| !id_sane(id)) {
        log("芯片 ID 异常，软件插拔后重读 ...");
        let _ = link.soft_unplug_gba();
        info = read_info(link);
    }
    if info.buffer_write_bytes > 0 {
        buf_wr = info.buffer_write_bytes as u16;
    }
    log(&format!("ID:{} 容量:{} BuffWr:{}", info.id_hex(), info.device_size, buf_wr));

    // 命中 profile 则用其命令序列做擦除（未命中走原硬编码，行为不变）。
    let prof = info.id.and_then(|id| {
        let all = profile::load_all();
        profile::match_by_id(&all, &id).map(|p| p.clone())
    });
    if let Some(p) = &prof {
        log(&format!("Profile: {} ({})", p.name, p.kind_label()));
    } else if info.id.as_ref().is_none_or(|id| !id_sane(id)) {
        log("未识别到有效 flash ID，中止烧录（请重插卡带）");
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    if opt.unlock_ppb {
        log("解锁 PPB (All PPB Erase) ...");
        unlock_all_ppb_logged(link, log);
        // PPB 命令集退出后再 F0，避免残留保护态影响擦写
        let _ = link.rom_write(0, &[0xf0, 0x00]);
    }

    if opt.no_erase {
        // --no-erase：跳过擦除，直接连续编程（仅用于测纯写入吞吐，要求 flash 已是擦除态）。
        log("跳过擦除，直接写入（--no-erase，flash 须已为擦除态）");
        let mut write_plog = ProgressLog::new(Phase::Write);
        let mut write_progress = |d: u64, t: u64| {
            progress(d, t);
            if write_plog.should_log(d, t) {
                log(&write_plog.format(d, t));
            }
        };
        if let Some(fail) =
            program_flow(link, rom, 0, length, buf_wr, &mut res, length, &mut write_progress)
        {
            log(&format!("写入失败 @0x{fail:08X}（已 DTR 重试×4）"));
            res.first_bad = Some(fail);
        }
    } else if opt.chip_erase {
        log("整片擦除 ...");
        // 对齐 WinForms/tmp：优先固件 0xf1（实测 ~1–2 分钟）；profile 软件擦仅作回落。
        // 若先走 profile，WAIT_TIMEOUT(30s) 不够大片擦完，会把 flash 留在擦除态再拖垮固件擦。
        // erase_chip_logged / chip_erase_profile_logged 在擦除期间发「已用秒/超时秒」心跳，
        // 让客户端进度条按时间线性推进，避免擦除阶段死停 0%。
        let ok = erase_chip_logged(link, CHIP_ERASE_TIMEOUT_SECS, progress, log)
            || match &prof {
                Some(p) => chip_erase_profile_logged(link, p, CHIP_ERASE_TIMEOUT_SECS, progress, log),
                None => false,
            };
        if !ok {
            log("整片擦除失败");
            res.first_bad = Some(0);
        } else {
            // 擦后稳定化：0xF0 复位 + 短延时。刚出擦除态的 flash 状态机可能残留
            // status 模式，立刻编程会在 0x0 处失败（DTR×4 挂，2026-08-15 实测）。
            link.rom_write(0, &[0xf0, 0x00]);
            std::thread::sleep(std::time::Duration::from_millis(100));
            log("开始写入（整片擦后连续编程，对齐 WinForms mission_programRom）");
            let mut write_attempt = 0u32;
            loop {
                write_attempt += 1;
                let mut write_plog = ProgressLog::new(Phase::Write);
                let mut write_progress = |d: u64, t: u64| {
                    progress(d, t);
                    if write_plog.should_log(d, t) {
                        log(&write_plog.format(d, t));
                    }
                };
                match program_flow(link, rom, 0, length, buf_wr, &mut res, length, &mut write_progress) {
                    None => break,
                    Some(fail) if write_attempt < 3 => {
                        // 写入失败：软插拔（断电重连清 flash/MCU 状态）后整段重写
                        log(&format!(
                            "写入失败 @0x{fail:08X}（第{write_attempt}次），软复位后重试整段写入 ..."
                        ));
                        let _ = link.soft_unplug_gba();
                        link.rom_write(0, &[0xf0, 0x00]);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Some(fail) => {
                        log(&format!("写入失败 @0x{fail:08X}（已重试整段×3）"));
                        res.first_bad = Some(fail);
                        break;
                    }
                }
            }
        }
    } else {
        // 默认快路径：**只擦 ROM 覆盖的扇区**（两线统一语义，原 --sector 开关已移除）。
        // 2026-08-15 起：原实现逐扇区擦全片（只是换擦法不省时）；现在 ROM 范围之外的
        // 旧内容会保留——需要彻底清场时用默认整片擦（GUI 勾选「全片清理」）。
        // 擦除和写入分别用独立 ProgressLog 上报进度。
        let erase_end = length;
        let total_erase_sectors = (erase_end + SECTOR_U64 - 1) / SECTOR_U64;
        log(&format!(
            "逐扇区擦除 ROM 范围（快路径, 0x{erase_end:X} B, {total_erase_sectors} 扇区）"
        ));
        let mut erase_plog = ProgressLog::new(Phase::Erase);
        erase_plog.report(0, total_erase_sectors, progress, log);
        let mut erase_ok = true;
        let mut off = 0u64;
        while off < erase_end {
            let ok = match &prof {
                Some(p) => {
                    sector_erase_profile(link, p, off as u32, 5) || erase_sector(link, off as u32, 5)
                }
                None => erase_sector(link, off as u32, 5),
            };
            if !ok {
                log(&format!("扇区 0x{off:08X} 擦除失败"));
                res.first_bad = Some(off);
                erase_ok = false;
                break;
            }
            off += SECTOR_U64;
            // 擦除进度：已擦扇区 / 芯片总扇区
            let done = off / SECTOR_U64;
            erase_plog.report(done, total_erase_sectors, progress, log);
        }

        if erase_ok {
            // 擦后稳定化 + 写入重试（同整片擦分支；刚出擦除态立刻编程会在 0x0 处失败）
            link.rom_write(0, &[0xf0, 0x00]);
            std::thread::sleep(std::time::Duration::from_millis(100));
            log(&format!("擦除完毕（{total_erase_sectors} 扇区），开始写入"));
            let mut write_attempt = 0u32;
            loop {
                write_attempt += 1;
                let mut write_plog = ProgressLog::new(Phase::Write);
                let mut write_progress = |d: u64, t: u64| {
                    progress(d, t);
                    if write_plog.should_log(d, t) {
                        log(&write_plog.format(d, t));
                    }
                };
                match program_flow(link, rom, 0, length, buf_wr, &mut res, length, &mut write_progress) {
                    None => break,
                    Some(fail) if write_attempt < 3 => {
                        log(&format!(
                            "写入失败 @0x{fail:08X}（第{write_attempt}次），软复位后重试整段写入 ..."
                        ));
                        let _ = link.soft_unplug_gba();
                        link.rom_write(0, &[0xf0, 0x00]);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Some(fail) => {
                        log(&format!("写入失败 @0x{fail:08X}（已重试整段×3）"));
                        res.first_bad = Some(fail);
                        break;
                    }
                }
            }
        }
    }

    if opt.verify && res.first_bad.is_none() {
        for round in 1..=8 {
            let (bad, mm) = find_bad_sectors(link, rom, length);
            res.mismatch_bytes = mm;
            log(&format!("校验(第{round}轮): {mm} 字节不符, {} 扇区", bad.len()));
            if bad.is_empty() {
                break;
            }
            let mut write_plog = ProgressLog::new(Phase::Write);
            for bsec in bad {
                let ok = match &prof {
                    Some(p) => {
                        sector_erase_profile(link, p, bsec as u32, 5)
                            || erase_sector(link, bsec as u32, 5)
                    }
                    None => erase_sector(link, bsec as u32, 5),
                };
                if ok {
                    let end = (bsec + SECTOR_U64).min(length);
                    let mut write_progress = |d: u64, t: u64| {
                        progress(d, t);
                        if write_plog.should_log(d, t) {
                            log(&write_plog.format(d, t));
                        }
                    };
                    if let Some(fail) = program_flow(
                        link,
                        rom,
                        bsec,
                        end,
                        buf_wr,
                        &mut res,
                        length,
                        &mut write_progress,
                    ) {
                        log(&format!("修复写入失败 @0x{fail:08X}"));
                    }
                } else {
                    log(&format!("修复: 扇区 0x{bsec:08X} 擦除失败"));
                }
            }
        }
    }

    res.success = res.first_bad.is_none() && res.mismatch_bytes == 0;
    res.seconds = start.elapsed().as_secs_f64();
    res
}
