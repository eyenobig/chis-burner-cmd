//! GBA S3511 RTC 读取（GPIO bit-bang，复刻自 mission_tools.cs）。
//!
//! GPIO 寄存器（字地址 = 字节地址 >> 1）：
//!   0xC4（字 0x62）数据端口  — SCK=bit0, SIO=bit1, CS=bit2
//!   0xC6（字 0x63）方向寄存器 — 1=输出, 0=输入
//!   0xC8（字 0x64）控制寄存器 — bit0=1 允许 GPIO 读写
//!
//! 命令 0xA6（读所有7个时间寄存器）LSB 先发；S3511 返回 7 字节 BCD：
//! year / month / date / day_of_week / hour / minute / second。

use crate::cartridge_link::CartridgeLink;

const SCK: u16 = 0x01; // bit 0
const SIO: u16 = 0x02; // bit 1
const CS:  u16 = 0x04; // bit 2

const ADDR_DATA: u32 = 0x62; // 0xC4 >> 1（字地址，rom_write 用）
const ADDR_DIR:  u32 = 0x63; // 0xC6 >> 1
const ADDR_CTRL: u32 = 0x64; // 0xC8 >> 1

fn gpio_write(link: &mut CartridgeLink, word_addr: u32, val: u16) {
    link.rom_write(word_addr, &[val as u8, (val >> 8) as u8]);
}

/// 发送 1 字节（LSB 先），调用期间 SIO 为输出方向。
fn send_byte(link: &mut CartridgeLink, mut value: u8) {
    gpio_write(link, ADDR_DIR, CS | SCK | SIO); // 全输出
    for _ in 0..8 {
        let bit = if (value & 0x01) != 0 { SIO } else { 0 };
        value >>= 1;
        gpio_write(link, ADDR_DATA, CS | bit);        // SCK=0, CS=1, SIO=bit
        gpio_write(link, ADDR_DATA, CS | SCK | bit);  // SCK=1, CS=1, SIO=bit（上升沿锁入）
    }
}

/// 接收 1 字节（LSB 先），SIO 切换为输入方向。
fn recv_byte(link: &mut CartridgeLink) -> u8 {
    gpio_write(link, ADDR_DIR, CS | SCK); // SIO=input，CS+SCK=output
    let mut value = 0u8;
    for _ in 0..8 {
        gpio_write(link, ADDR_DATA, CS);           // SCK=0（下降沿，S3511 输出数据）
        gpio_write(link, ADDR_DATA, CS | SCK);     // SCK=1（数据稳定）
        let mut buf = [0u8; 2];
        link.rom_read(0xC4, &mut buf);             // 读 GPIO 数据端口（字节地址）
        // LSB 先：当前 bit 对应结果字节的最高位，逐步右移积累
        value >>= 1;
        if (buf[0] & (SIO as u8)) != 0 {
            value |= 0x80;
        }
    }
    value
}

/// BCD 字节转十进制（去掉无效高位后）。
fn bcd(b: u8) -> u8 {
    (b & 0x0f) + ((b >> 4) & 0x07) * 10
}

pub struct RtcTimeGba {
    pub year: u16,       // 2000+BCD
    pub month: u8,       // 1-12
    pub date: u8,        // 1-31
    pub day_of_week: u8, // 0-6
    pub hour: u8,        // 0-23
    pub minute: u8,      // 0-59
    pub second: u8,      // 0-59
}

/// 读取 S3511 全部时间寄存器。失败（无 GPIO 功能）返回 None。
pub fn read_s3511(link: &mut CartridgeLink) -> Option<RtcTimeGba> {
    // 使能 GPIO
    gpio_write(link, ADDR_CTRL, 0x0001);
    // 初始状态：全输出，SCK=1, CS=0（空闲）
    gpio_write(link, ADDR_DIR, CS | SCK | SIO);
    gpio_write(link, ADDR_DATA, SCK);

    // 发读命令 0xA6（读全部7寄存器，LSB 先）
    send_byte(link, 0xA6);

    // 读 7 字节：year/month/date/day_of_week/hour/minute/second
    let year_raw  = recv_byte(link);
    let month_raw = recv_byte(link) & 0x1f;
    let date_raw  = recv_byte(link) & 0x3f;
    let dow_raw   = recv_byte(link) & 0x07;
    let hour_raw  = recv_byte(link) & 0x3f;
    let min_raw   = recv_byte(link) & 0x7f;
    let sec_raw   = recv_byte(link) & 0x7f;

    // CS 释放（SCK=1, CS=0）
    gpio_write(link, ADDR_DATA, SCK);
    // 关闭 GPIO
    gpio_write(link, ADDR_CTRL, 0x0000);

    Some(RtcTimeGba {
        year:        bcd(year_raw) as u16 + 2000,
        month:       bcd(month_raw),
        date:        bcd(date_raw),
        day_of_week: bcd(dow_raw),
        hour:        bcd(hour_raw),
        minute:      bcd(min_raw),
        second:      bcd(sec_raw),
    })
}
