//! MBC · 写：编程 / 烧录 GB/GBC ROM（MBC3 / MBC5 自动识别）。
//!
//! 流程对齐 tmp_gb_burn（本机 COM13 成功路径）/ beggar_socket：
//! 软件插拔 **3.3V** → CFI/ID → **整片擦 + ROM 范围扇区擦（16KB）** → 再软件插拔 →
//! 逐 bank 0xfc 编程 → 读回校验。
//! MBC5 bank：N→N+1 @0x2000（本机卡实测，见 `switch_bank`）。
#![allow(dead_code)]

use std::time::Instant;

use super::delete::erase_range_logged;
use super::read::{bus_addr, is_js28f256, rom_get_cfi, rom_get_id, switch_bank};
use crate::cartridge_link::CartridgeLink;
use crate::progress_display::{Phase, ProgressLog};
use crate::rom::gba::data::BurnResult;
use crate::rom::mbc::data::{mbc_name, MbcKind};

const PACKET: usize = 256;
/// 本机成功路径扇区粒度；CFI 报 64KiB 过粗时用此值。
const ERASE_SECTOR_16K: u32 = 16 * 1024;

/// 烧录 GB/GBC ROM：识别 → 查 flash → 空间校验 → 擦除 → 写入 → 校验。
/// 调用前 `open_powered` 默认 3.3V；此处 `soft_unplug_3v3` 保持 3.3V 时序做软件插拔。
/// 命令结束后由调用方 `power_idle` 确认 3.3V。
///
/// `chip_erase`：MBC 默认已走「整片 + 扇区」；该标志保留与 GBA/CLI 对齐（true 时强制整片优先）。
pub fn burn(
    link: &mut CartridgeLink,
    rom: &[u8],
    verify: bool,
    chip_erase: bool,
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
    let elapsed = |s: &Instant| format!("{:.1}s", s.elapsed().as_secs_f64());

    // 每次 mission：关口再开 + 3.3V 断电/上电（软件等效插拔，避免连续操作残留）
    if let Err(e) = link.soft_unplug_3v3() {
        log(&format!("软件复位失败: {e}"));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    // ---- 步骤 1：识别 MBC 代次 ----
    // 卡上 0x147 在 MBC5 窗口是 0x4147；裸读 0x147 常为 0xFF → 默认 MBC5。
    let file_ct = rom.get(0x147).copied().unwrap_or(0xFF);
    let live_ct = super::read::read_cart_byte(link, 0x147).unwrap_or(0xFF);
    let kind = match live_ct {
        0x0F..=0x13 => MbcKind::Mbc3,
        0x19..=0x1E => MbcKind::Mbc5,
        _ => MbcKind::Mbc5,
    };
    log(&format!(
        "识别: ROM=0x{:02X} {} / cart=0x{:02X} -> {}",
        file_ct,
        mbc_name(file_ct),
        live_ct,
        kind.label()
    ));

    // ---- 步骤 2：CFI + Autoselect ID ----
    let (device_size, buf_wr_cfi, cfi_sector) = rom_get_cfi(link);
    let id = rom_get_id(link);
    let mut buf_wr: u16 = if buf_wr_cfi == 0 { 32 } else { buf_wr_cfi };
    if is_js28f256(&id) {
        buf_wr = 256;
        log("JS28F256：buf_wr=256");
    }
    // 扇区：允许 16KB；CFI 缺失或 >16KB（过粗）时用 16KB（tmp_gb_burn 成功路径）
    let sector_size = if cfi_sector >= 0x1000 && cfi_sector <= ERASE_SECTOR_16K {
        cfi_sector
    } else {
        ERASE_SECTOR_16K
    };
    log(&format!(
        "flash id={:02X}{:02X}{:02X}{:02X} size={} buf={} sector={} (cfi_sec={})",
        id[0],
        id[1],
        id[2],
        id[3],
        if device_size == 0 {
            "?".to_string()
        } else {
            device_size.to_string()
        },
        buf_wr,
        sector_size,
        cfi_sector
    ));

    // ---- 步骤 3：空间校验 ----
    if device_size > 0 && length > device_size {
        log(&format!("空间不足: ROM {} > flash {}", length, device_size));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    link.gbc_write(0x00, &[0xf0]);
    link.gbc_warm_up();
    switch_bank(link, 0, kind);

    // ---- 步骤 4：整片擦 + ROM 范围扇区擦（对齐 tmp；不因「看似空白」跳过）----
    let force_skip_erase = std::env::var_os("CFB_SKIP_ERASE").is_some();
    // MBC：默认始终整片+扇区；`chip_erase` 为 false 时仍走同一路径（临时成功行为优先）
    let _ = chip_erase;
    if force_skip_erase {
        log("CFB_SKIP_ERASE：跳过擦除");
    } else {
        log("擦除：整片 + ROM 范围扇区补擦 ...");
        // 整片失败不硬退：仍依赖后续扇区擦（tmp 同策略）
        if !super::delete::erase_chip_logged(link, 180, progress, log) {
            log("整片擦失败或未净；将仅依赖后续扇区擦");
        }
        log(&format!(
            "ROM 范围扇区擦（{}B）...",
            sector_size
        ));
        if !erase_range_logged(link, kind, 0, length, sector_size, progress, log) {
            log(&format!("扇区擦失败 | {}", elapsed(&start)));
            res.first_bad = Some(0);
            res.seconds = start.elapsed().as_secs_f64();
            return res;
        }
        link.gbc_write(0x00, &[0xf0]);
        switch_bank(link, 0, kind);
        // 擦后空白：警告可继续（对齐 tmp），不硬拦编程
        let banks = ((length + 0x3fff) / 0x4000) as u32;
        let mut probe = [0u8; 16];
        let mut dirty = false;
        for b in 0..banks.min(32) {
            switch_bank(link, b, kind);
            if !(link.gbc_read(bus_addr(b << 14, kind), &mut probe)
                && probe.iter().all(|&x| x == 0xff))
            {
                dirty = true;
                log(&format!("警告：擦后 bank{b} 非空，仍继续编程"));
                break;
            }
        }
        switch_bank(link, 0, kind);
        if !dirty {
            log("擦后空白抽查通过");
        }
    }

    // 擦除后再软件插拔：对齐「擦除 mission 结束关口 → 烧录 mission 重开」；
    // 同会话硬扛易出现编程 NAK（物理插拔可好）。
    log("擦后软件插拔（关电→关串口→3.3V）...");
    if let Err(e) = link.soft_unplug_3v3() {
        log(&format!("擦后软件复位失败: {e}"));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }
    let (_ds, buf_wr_re, _sec) = rom_get_cfi(link);
    let id2 = rom_get_id(link);
    log(&format!(
        "擦后重识别 id={:02X}{:02X}{:02X}{:02X} buf={}",
        id2[0], id2[1], id2[2], id2[3], buf_wr_re
    ));
    if buf_wr_re != 0 {
        buf_wr = buf_wr_re;
    }
    if is_js28f256(&id2) {
        buf_wr = 256;
    }
    switch_bank(link, 0, kind);
    std::thread::sleep(std::time::Duration::from_millis(50));

    // ---- 步骤 5：写入 ----
    log(&format!("开始写入 ... buf_wr={buf_wr}"));
    {
        let mut write_plog = ProgressLog::new(Phase::Write);
        let mut write_progress = |d: u64, t: u64| {
            progress(d, t);
            if write_plog.should_log(d, t) {
                log(&write_plog.format(d, t));
            }
        };
        if let Some(bad) = program_flow(
            link,
            kind,
            rom,
            0,
            length,
            buf_wr,
            &mut res,
            length,
            &mut write_progress,
        ) {
            log(&format!("写入失败 @0x{bad:X} | {}", elapsed(&start)));
            res.first_bad = Some(bad);
            res.seconds = start.elapsed().as_secs_f64();
            return res;
        }
    }
    link.gbc_write(0x4000, &[0x00]);

    // ---- 步骤 6：读回校验；残留 FF 可补写第二遍（无需再擦）----
    if verify {
        log("校验中 ...");
        let mm = {
            let mut verify_plog = ProgressLog::new(Phase::Verify);
            let mut verify_progress = |d: u64, t: u64| {
                progress(d, t);
                if verify_plog.should_log(d, t) {
                    log(&verify_plog.format(d, t));
                }
            };
            let (mm, first_msg) =
                verify_flow(link, kind, rom, length, &mut verify_progress, &mut res);
            if let Some(msg) = first_msg {
                log(&msg);
            }
            mm
        };
        res.mismatch_bytes = mm;
        if mm > 0 {
            log(&format!("校验: {mm} 字节不符，补写 ..."));
            res.first_bad = None;
            res.mismatch_bytes = 0;
            link.gbc_write(0x00, &[0xf0]);
            switch_bank(link, 0, kind);
            let rewrite_fail = {
                let mut rewrite_plog = ProgressLog::new(Phase::Write);
                let mut rewrite_progress = |d: u64, t: u64| {
                    progress(d, t);
                    if rewrite_plog.should_log(d, t) {
                        log(&rewrite_plog.format(d, t));
                    }
                };
                program_flow(
                    link,
                    kind,
                    rom,
                    0,
                    length,
                    buf_wr,
                    &mut res,
                    length,
                    &mut rewrite_progress,
                )
            };
            if let Some(bad) = rewrite_fail {
                log(&format!("补写失败 @0x{bad:X}"));
                res.first_bad = Some(bad);
            } else {
                let mm2 = {
                    let mut verify2_plog = ProgressLog::new(Phase::Verify);
                    let mut verify2_progress = |d: u64, t: u64| {
                        progress(d, t);
                        if verify2_plog.should_log(d, t) {
                            log(&verify2_plog.format(d, t));
                        }
                    };
                    let (mm2, first_msg) =
                        verify_flow(link, kind, rom, length, &mut verify2_progress, &mut res);
                    if let Some(msg) = first_msg {
                        log(&msg);
                    }
                    mm2
                };
                res.mismatch_bytes = mm2;
                log(&format!("补写后校验: {mm2} 字节不符 | {}", elapsed(&start)));
            }
        } else {
            log(&format!("校验: 0 字节不符 | {}", elapsed(&start)));
        }
    }

    res.success = res.first_bad.is_none() && res.mismatch_bytes == 0;
    res.seconds = start.elapsed().as_secs_f64();
    res
}

/// 编程区间；跨 bank 前 flash 复位。重连用 3.3V + gbc_warm_up。
pub(crate) fn program_range(
    link: &mut CartridgeLink,
    kind: MbcKind,
    data: &[u8],
    rom_base: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Option<u64> {
    let mut res = BurnResult {
        success: false,
        bytes_written: 0,
        reconnects: 0,
        first_bad: None,
        mismatch_bytes: 0,
        seconds: 0.0,
    };
    let total = data.len() as u64;
    let mut written = 0u64;
    let mut current_bank: i64 = -1;
    while written < total {
        let len = ((total - written) as usize).min(PACKET);
        let pk = &data[written as usize..written as usize + len];
        let rom_off = (rom_base + written) as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            if current_bank >= 0 {
                link.gbc_write(0x00, &[0xf0]);
            }
            current_bank = bank;
            switch_bank(link, bank as u32, kind);
        }
        let cartridge_addr = bus_addr(rom_off, kind);
        let mut tries = 0;
        loop {
            if link.gbc_rom_program(cartridge_addr, pk, 32) {
                break;
            }
            tries += 1;
            if tries % 5 == 0 {
                res.reconnects += 1;
                let _ = link.reconnect_as(true);
                switch_bank(link, bank as u32, kind);
            }
            if tries >= 60 {
                return Some(rom_base + written);
            }
        }
        written += len as u64;
        progress(written, total);
    }
    let _ = res;
    None
}

fn program_flow(
    link: &mut CartridgeLink,
    kind: MbcKind,
    rom: &[u8],
    from: u64,
    to: u64,
    buf_wr: u16,
    res: &mut BurnResult,
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Option<u64> {
    let mut written = from;
    let mut current_bank: i64 = -1;
    let align = (buf_wr as usize).max(1);
    let mut prefer_chunk = PACKET - (PACKET % align);
    let mut prefer_buf = buf_wr;
    while written < to {
        let align_off = (written as usize) % align;
        let mut chunk = if align_off != 0 {
            (align - align_off).min((to - written) as usize)
        } else {
            ((to - written) as usize).min(prefer_chunk)
        };
        if chunk == 0 {
            chunk = 1;
        }
        let rom_off = written as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            if current_bank >= 0 {
                link.gbc_write(0x00, &[0xf0]);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            current_bank = bank;
            prefer_chunk = PACKET - (PACKET % align);
            prefer_buf = buf_wr;
            if align_off == 0 {
                chunk = ((to - written) as usize).min(prefer_chunk);
            }
            switch_bank(link, bank as u32, kind);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let mut tries = 0;
        let mut use_buf = if chunk == 1 { 0 } else { prefer_buf };
        loop {
            let pk = &rom[written as usize..written as usize + chunk];
            let cartridge_addr = bus_addr(rom_off, kind);
            if link.gbc_rom_program(cartridge_addr, pk, use_buf) {
                if chunk >= 32 {
                    prefer_chunk = chunk.min(PACKET);
                }
                prefer_buf = if chunk == 1 { 0 } else { use_buf };
                break;
            }
            tries += 1;
            if tries == 1 {
                eprintln!(
                    "cfb: program nak @rom=0x{rom_off:X} bus=0x{cartridge_addr:X} bank={bank} chunk={chunk} buf={use_buf}"
                );
            }
            link.gbc_write(0x00, &[0xf0]);
            std::thread::sleep(std::time::Duration::from_millis(2));
            switch_bank(link, bank as u32, kind);
            if chunk > align {
                chunk = align;
                use_buf = buf_wr;
            } else if chunk > 1 {
                chunk = 1;
                use_buf = 0;
            }
            prefer_chunk = chunk;
            if tries % 12 == 0 {
                res.reconnects += 1;
                let _ = link.reconnect_as(true);
                switch_bank(link, bank as u32, kind);
            }
            if tries >= 25 {
                return Some(written);
            }
        }

        written += chunk as u64;
        res.bytes_written += chunk as u64;
        progress(written, total);
    }
    None
}

fn verify_flow(
    link: &mut CartridgeLink,
    kind: MbcKind,
    rom: &[u8],
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
    res: &mut BurnResult,
) -> (u32, Option<String>) {
    let mut mismatch = 0u32;
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut buf = vec![0u8; PACKET];
    let mut first_msg = None;
    while read < total {
        let n = ((total - read) as usize).min(PACKET);
        let rom_off = read as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            if current_bank >= 0 {
                link.gbc_write(0x00, &[0xf0]);
            }
            current_bank = bank;
            switch_bank(link, bank as u32, kind);
        }
        let cartridge_addr = bus_addr(rom_off, kind);
        let b = &mut buf[..n];
        if !link.gbc_read(cartridge_addr, b) {
            res.reconnects += 1;
            let _ = link.reconnect_as(true);
            switch_bank(link, bank as u32, kind);
            continue;
        }
        for i in 0..n {
            if b[i] != rom[read as usize + i] {
                mismatch += 1;
                if res.first_bad.is_none() {
                    res.first_bad = Some(read + i as u64);
                    first_msg = Some(format!(
                        "0x{:08X} 校验失败: {:02X} → {:02X}",
                        read as u64 + i as u64,
                        rom[read as usize + i],
                        b[i]
                    ));
                }
            }
        }
        read += n as u64;
        progress(read, total);
    }
    (mismatch, first_msg)
}
