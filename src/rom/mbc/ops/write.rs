//! MBC · 写：编程 / 烧录 GB/GBC ROM（MBC3 / MBC5 自动识别）。
//!
//! 流程对齐 beggar_socket `mission_programRom_mbc5`：
//! 5V 重上电 → CFI →（非空白则）扇区擦除 → 逐 bank 0xfc 编程 → 读回校验。
//! MBC5 bank：N→N+1 @0x2100（本机卡实测，见 `switch_bank`）。
#![allow(dead_code)]

use std::time::Instant;

use super::delete::erase_range_logged;
use super::read::{bus_addr, rom_get_cfi, switch_bank};
use crate::cartridge_link::CartridgeLink;
use crate::rom::gba::data::BurnResult;
use crate::rom::mbc::data::{mbc_name, MbcKind};

const PACKET: usize = 256;

/// 烧录 GB/GBC ROM：识别 → 查 flash → 空间校验 → 擦除 → 写入 → 校验。
/// 调用前应已上 5V 电（`device::power(link, Voltage::V5)`）；此处再做一次 5V 重上电对齐 C#。
pub fn burn(
    link: &mut CartridgeLink,
    rom: &[u8],
    verify: bool,
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

    // 对齐 C#：5V 断电再上电稳定总线
    link.power(0);
    std::thread::sleep(std::time::Duration::from_millis(111));
    link.power(2); // 5V
    std::thread::sleep(std::time::Duration::from_millis(333));
    link.gbc_warm_up();

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

    // ---- 步骤 2：CFI ----
    let (device_size, buf_wr_cfi, _cfi_sector) = rom_get_cfi(link);
    // 擦净后用 CFI 缓冲写；未擦净时缓冲写会大面积 NAK。
    let buf_wr: u16 = if buf_wr_cfi == 0 { 32 } else { buf_wr_cfi };
    let sector_size = super::read::BANK_SIZE;
    if device_size == 0 {
        log(&format!(
            "flash: size=? rom={} sector={} buf={}",
            length, sector_size, buf_wr
        ));
    } else {
        log(&format!(
            "flash: size={} buf={} sector={}",
            device_size, buf_wr, sector_size
        ));
    }

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

    // ---- 步骤 4：按 ROM 覆盖的 bank 判断是否需擦；优先整片擦 ----
    log("擦除目标区 ...");
    let banks = ((length + 0x3fff) / 0x4000) as u32;
    let force_skip_erase = std::env::var_os("CFB_SKIP_ERASE").is_some();
    let mut need_erase = false;
    if !force_skip_erase {
        let mut sample = [0u8; 64];
        for b in 0..banks {
            switch_bank(link, b, kind);
            if !(link.gbc_read(bus_addr(b << 14, kind), &mut sample)
                && sample.iter().all(|&x| x == 0xff))
            {
                need_erase = true;
                break;
            }
        }
        switch_bank(link, 0, kind);
    } else {
        log("CFB_SKIP_ERASE：跳过擦除");
    }
    if need_erase {
        log("整片擦除中...");
        progress(0, 1);
        let chip_ok = super::delete::erase_chip(link, 240);
        if chip_ok {
            progress(1, 1);
            log(&format!("整片擦除完成 | {}", elapsed(&start)));
        } else {
            log("整片擦失败，改扇区擦...");
            if !erase_range_logged(link, kind, 0, length, sector_size, progress, log) {
                res.first_bad = Some(0);
                res.seconds = start.elapsed().as_secs_f64();
                return res;
            }
        }
    } else {
        log("目标区已空白，跳过擦除");
    }
    link.gbc_write(0x00, &[0xf0]);
    switch_bank(link, 0, kind);
    // 逐 bank 验空（跳过擦除时不验）
    if !force_skip_erase {
        let mut probe = [0u8; 64];
        let mut dirty = false;
        for b in 0..banks {
            switch_bank(link, b, kind);
            if !(link.gbc_read(bus_addr(b << 14, kind), &mut probe)
                && probe.iter().all(|&x| x == 0xff))
            {
                dirty = true;
                log(&format!("擦后 bank{b} 非空"));
                break;
            }
        }
        switch_bank(link, 0, kind);
        if dirty {
            res.first_bad = Some(0);
            res.seconds = start.elapsed().as_secs_f64();
            return res;
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(80));

    // ---- 步骤 5：写入 ----
    log("开始写入 ...");
    if let Some(bad) = program_flow(link, kind, rom, 0, length, buf_wr, &mut res, length, progress) {
        log(&format!("写入失败 @0x{bad:X} | {}", elapsed(&start)));
        res.first_bad = Some(bad);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }
    link.gbc_write(0x4000, &[0x00]);

    // ---- 步骤 6：读回校验；残留 FF 可补写第二遍（无需再擦）----
    if verify {
        log("校验中 ...");
        let mm = verify_flow(link, kind, rom, length, progress, log, &mut res);
        res.mismatch_bytes = mm;
        if mm > 0 && res.first_bad.is_some() {
            // first_bad 在 verify 里可能被设置；补写前清掉编程失败标记语义
        }
        if mm > 0 {
            log(&format!("校验: {mm} 字节不符，补写 ..."));
            res.first_bad = None;
            res.mismatch_bytes = 0;
            link.gbc_write(0x00, &[0xf0]);
            switch_bank(link, 0, kind);
            if let Some(bad) =
                program_flow(link, kind, rom, 0, length, buf_wr, &mut res, length, progress)
            {
                log(&format!("补写失败 @0x{bad:X}"));
                res.first_bad = Some(bad);
            } else {
                let mm2 = verify_flow(link, kind, rom, length, progress, log, &mut res);
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

/// 编程区间；跨 bank 前 flash 复位。重连用 5V + gbc_warm_up。
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
    let mut prefer_chunk = PACKET;
    let mut prefer_buf = buf_wr;
    while written < to {
        let mut chunk = ((to - written) as usize).min(prefer_chunk);
        let rom_off = written as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            if current_bank >= 0 {
                link.gbc_write(0x00, &[0xf0]);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            current_bank = bank;
            prefer_chunk = PACKET;
            prefer_buf = buf_wr;
            chunk = ((to - written) as usize).min(prefer_chunk);
            switch_bank(link, bank as u32, kind);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let mut tries = 0;
        let mut use_buf = prefer_buf;
        loop {
            let pk = &rom[written as usize..written as usize + chunk];
            let cartridge_addr = bus_addr(rom_off, kind);
            if link.gbc_rom_program(cartridge_addr, pk, use_buf) {
                prefer_chunk = chunk.max(1);
                prefer_buf = use_buf;
                break;
            }
            tries += 1;
            if tries == 1 {
                eprintln!(
                    "cfb: program nak @rom=0x{rom_off:X} bus=0x{cartridge_addr:X} bank={bank} chunk={chunk}"
                );
            }
            link.gbc_write(0x00, &[0xf0]);
            std::thread::sleep(std::time::Duration::from_millis(2));
            switch_bank(link, bank as u32, kind);
            if chunk > 32 {
                chunk = 32;
            } else if chunk > 8 {
                chunk = 8;
            } else if chunk > 1 {
                chunk = 1;
            }
            use_buf = 0;
            prefer_chunk = chunk;
            prefer_buf = 0;
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
    log: &mut dyn FnMut(&str),
    res: &mut BurnResult,
) -> u32 {
    let mut mismatch = 0u32;
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut buf = vec![0u8; PACKET];
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
                    log(&format!(
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
    mismatch
}
