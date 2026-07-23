//! GBA · 写：编程 / 烧录 ROM。
//!
//! 从参考源 `GbaFlasher.cs` 的 `ProgramFlow` / `FindBadSectors` / `Burn` 复刻。
//! 健壮性：每包必须 ACK 才前进，连续无应答则 `reconnect` 复活后重试；烧后逐扇区校验+修复。
//! ⚠️ 未经硬件测试（见根目录 TODO.md）。

use std::time::Instant;

use super::delete::{chip_erase_profile, erase_chip, erase_sector, sector_erase_profile, unlock_all_ppb};
use super::read::read_info;
use crate::cartridge_link::CartridgeLink;
use crate::profile;
use crate::rom::gba::data::{BurnOptions, BurnResult};

pub const SECTOR: u64 = 0x20000; // 128KB
const PACKET: usize = 4096;

/// 编程 [from,to)；每包必须 ACK 才前进，连续失败重连复活。返回首个失败地址或 None(完成)。
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
    while pos < to {
        let len = ((to - pos) as usize).min(PACKET);
        let pk = &rom[pos as usize..pos as usize + len];

        let mut tries = 0;
        loop {
            if link.rom_program(pos as u32, pk, buf_wr) {
                break;
            }
            tries += 1;
            if tries % 5 == 0 {
                res.reconnects += 1;
                let _ = link.reconnect();
            }
            if tries >= 60 {
                return Some(pos);
            }
        }

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
            let _ = link.reconnect();
            continue;
        }
        for i in 0..len {
            if b[i] != rom[pos as usize + i] {
                bad.insert(((pos + i as u64) / SECTOR) * SECTOR);
                mismatch += 1;
            }
        }
        pos += len as u64;
    }
    (bad.into_iter().collect(), mismatch)
}

/// 完整烧录：(可选解锁PPB) → (整片或逐扇区)擦除+编程 → 校验+修复。
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

    let mut buf_wr: u16 = 32; // S29GL256 默认；CFI 可覆盖
    let info = read_info(link);
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
    }

    if opt.unlock_ppb {
        log("解锁 PPB (All PPB Erase) ...");
        unlock_all_ppb(link);
    }

    if opt.chip_erase {
        log("整片擦除 ...");
        let ok = match &prof {
            Some(p) => chip_erase_profile(link, p, 200),
            None => erase_chip(link, 200),
        };
        if !ok {
            res.first_bad = Some(0);
        } else if let Some(fail) = program_flow(link, rom, 0, length, buf_wr, &mut res, length, progress) {
            res.first_bad = Some(fail);
        }
    } else {
        let mut b = 0u64;
        while b < length {
            let end = (b + SECTOR).min(length);
            let ok = match &prof {
                Some(p) => sector_erase_profile(link, p, b as u32, 5),
                None => erase_sector(link, b as u32, 5),
            };
            if !ok {
                log(&format!("扇区 0x{b:08X} 擦除失败"));
                res.first_bad = Some(b);
                break;
            }
            if let Some(fail) = program_flow(link, rom, b, end, buf_wr, &mut res, length, progress) {
                res.first_bad = Some(fail);
                break;
            }
            b += SECTOR;
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
            for bsec in bad {
                let ok = match &prof {
                    Some(p) => sector_erase_profile(link, p, bsec as u32, 5),
                    None => erase_sector(link, bsec as u32, 5),
                };
                if ok {
                    let end = (bsec + SECTOR).min(length);
                    program_flow(link, rom, bsec, end, buf_wr, &mut res, length, progress);
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
