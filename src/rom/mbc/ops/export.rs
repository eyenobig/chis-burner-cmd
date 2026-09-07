//! MBC · 导：导出（dump GB/GBC ROM 到文件，MBC5）。
//!
//! 从参考源 `mission_mbc5.cs` 的 `mission_dumpRom_mbc5` 复刻：按 16KB bank 切换，
//! 每 4096B 用 `gbc_read`（cmd 0xfb）读出。长度由调用方给定（通常按 ROM 头容量）。

use std::fs::File;
use std::io::Write;

use super::read::{bus_addr, switch_bank_mbcx};
use crate::cartridge_link::CartridgeLink;
use crate::rom::mbc::data::MbcKind;

const PACKET: usize = 4096;

/// 导出 `len` 字节 GB/GBC ROM 到 `path`。调用前应已上电（默认 3.3V）。
pub fn dump(
    link: &mut CartridgeLink,
    kind: MbcKind,
    len: u64,
    path: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> std::io::Result<()> {
    let mut f = File::create(path)?;
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut flash_bank: i32 = -1;
    let mut buf = vec![0u8; PACKET];
    while read < len {
        let n = ((len - read) as usize).min(PACKET);
        let rom_off = read as u32;
        let bank = (rom_off >> 14) as i64;
        // MBC5：bank0 固定区 0x0000-0x3FFF 直读（reg1 窗口=page1，读不到 bank0）；
        // 主区 reg=N 直读（P(R)=R 标准 MBC5，与 0.4.1 烧录「bank N→page N」对齐；
        // 旧 N+1 映射整卷错一页，实测 dump[N]=ROM[N+1]）。
        // 其余代次（MBC1/2/3）维持 switch_bank + bus_addr 原路径。
        let bank0_fixed = kind == MbcKind::Mbc5 && bank == 0;
        if bank != current_bank {
            current_bank = bank;
            if kind == MbcKind::Mbc5 {
                if bank >= 1 {
                    let reg = bank as u32;
                    link.gbc_write(0x3000, &[((reg >> 8) & 0xff) as u8]);
                    link.gbc_write(0x2000, &[(reg & 0xff) as u8]);
                }
            } else {
                switch_bank_mbcx(link, bank as u32, kind, &mut flash_bank);
            }
        }
        let cartridge_addr = if bank0_fixed {
            rom_off & 0x3fff
        } else {
            bus_addr(rom_off, kind)
        };
        let b = &mut buf[..n];
        if !link.gbc_read(cartridge_addr, b) {
            let _ = link.reconnect_as(true);
            flash_bank = -1;
            if kind == MbcKind::Mbc5 {
                if bank >= 1 {
                    let reg = bank as u32;
                    link.gbc_write(0x3000, &[((reg >> 8) & 0xff) as u8]);
                    link.gbc_write(0x2000, &[(reg & 0xff) as u8]);
                }
            } else {
                switch_bank_mbcx(link, bank as u32, kind, &mut flash_bank);
            }
            current_bank = bank;
            continue;
        }
        f.write_all(b)?;
        read += n as u64;
        progress(read, len);
    }
    f.flush()
}
