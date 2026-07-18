//! MBC3 RTC 读取（内存映射寄存器，复刻自 mission_tools.cs）。
//!
//! 时序：
//!   1. 使能 RAM：0x0A → 0x0000
//!   2. 锁存当前时间：0x00 → 0x6000，再 0x01 → 0x6000
//!   3. 逐个选通寄存器（0x08-0x0C → 0x4000），从 0xA000 读取
//!   4. 寄存器含义：
//!      0x08 秒（0-59）  0x09 分（0-59）  0x0A 时（0-23）
//!      0x0B 天低8位     0x0C bit0=天bit8, bit6=停止, bit7=溢出

use crate::cartridge_link::CartridgeLink;

pub struct RtcTimeMbc3 {
    pub second:    u8,
    pub minute:    u8,
    pub hour:      u8,
    pub day_count: u16,  // 9 位天计数（0-511）
    pub halted:    bool,
    pub overflow:  bool,
}

/// 读取 MBC3 RTC 寄存器。
pub fn read_mbc3_rtc(link: &mut CartridgeLink) -> Option<RtcTimeMbc3> {
    // 使能 RAM
    link.gbc_write(0x0000, &[0x0A]);
    // 锁存时间（0x00 → 0x01）
    link.gbc_write(0x6000, &[0x00]);
    link.gbc_write(0x6000, &[0x01]);

    // 读 5 个 RTC 寄存器
    let mut vals = [0u8; 5];
    for (i, reg) in (0x08u8..=0x0Cu8).enumerate() {
        link.gbc_write(0x4000, &[reg]);
        let mut buf = [0u8; 1];
        link.gbc_read(0xA000, &mut buf);
        vals[i] = buf[0];
    }

    // 解锁（恢复）
    link.gbc_write(0x6000, &[0x00]);
    // 禁用 RAM（可选，保持一致性）
    link.gbc_write(0x0000, &[0x00]);

    let day_count = (vals[3] as u16) | (((vals[4] & 0x01) as u16) << 8);
    let halted    = (vals[4] & 0x40) != 0;
    let overflow  = (vals[4] & 0x80) != 0;

    Some(RtcTimeMbc3 {
        second: vals[0],
        minute: vals[1],
        hour:   vals[2],
        day_count,
        halted,
        overflow,
    })
}
