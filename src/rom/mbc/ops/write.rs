//! MBC · 写：编程 / 烧录 GB/GBC ROM（MBC5）。
//!
//! 从参考源 `mission_mbc5.cs` 的 `mission_programRom_mbc5` 复刻：按 16KB bank 切换，
//! 每 4096B 用 `gbc_rom_program`（cmd 0xfc）编程，bufferWriteBytes 取自 CFI。
//! ⚠️ 未经硬件测试（见根目录 TODO.md）。
#![allow(dead_code)]

use std::time::Instant;

use super::delete::erase_range;
use super::read::{bus_addr, rom_get_size, switch_bank};
use crate::cartridge_link::CartridgeLink;
use crate::rom::gba::data::BurnResult;

const PACKET: usize = 4096;

/// 烧录 GB/GBC ROM：CFI 取写缓冲 → 擦除目标区 → 逐 bank 编程。
/// 调用前应已上 5V 电（`device::power(link, Voltage::V5)`）。
pub fn burn(
    link: &mut CartridgeLink,
    rom: &[u8],
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

    let (device_size, buf_wr) = rom_get_size(link);
    let buf_wr = if buf_wr == 0 { 32 } else { buf_wr };
    log(&format!("MBC5 容量:{device_size} BuffWr:{buf_wr}"));

    // 先擦除目标区。
    log("擦除目标区 ...");
    if !erase_range(link, 0, length) {
        log("擦除失败");
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    // 逐 bank 编程。
    let mut written = 0u64;
    let mut current_bank: i64 = -1;
    while written < length {
        let len = ((length - written) as usize).min(PACKET);
        let pk = &rom[written as usize..written as usize + len];
        let rom_off = written as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            current_bank = bank;
            switch_bank(link, bank as u32);
        }
        let cartridge_addr = bus_addr(rom_off);

        // 0xfc 单发不重试；这里补一层重连重试以提健壮性。
        let mut tries = 0;
        loop {
            if link.gbc_rom_program(cartridge_addr, pk, buf_wr) {
                break;
            }
            tries += 1;
            if tries % 5 == 0 {
                res.reconnects += 1;
                let _ = link.reconnect();
                switch_bank(link, bank as u32);
            }
            if tries >= 60 {
                res.first_bad = Some(written);
                res.seconds = start.elapsed().as_secs_f64();
                return res;
            }
        }

        written += len as u64;
        res.bytes_written += len as u64;
        progress(written, length);
    }

    link.gbc_write(0x4000, &[0x00]); // settle bank reg

    res.success = true;
    res.seconds = start.elapsed().as_secs_f64();
    res
}
