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

/// BCD 字节转十进制（调用前须先过 `bcd_valid`）。
fn bcd(b: u8) -> u8 {
    (b & 0x0f) + ((b >> 4) & 0x0f) * 10
}

/// 两个 nibble 都 ≤ 9 才是合法 BCD。
fn bcd_valid(b: u8) -> bool {
    (b & 0x0f) <= 9 && ((b >> 4) & 0x0f) <= 9
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

/// 各寄存器的有效位掩码（year/month/date/dow/hour/min/sec）。
/// hour 的 bit6 是 12 小时制的 AM/PM 标志，掩掉后按 24 小时制取值。
const REG_MASKS: [u8; 7] = [0xff, 0x1f, 0x3f, 0x07, 0x3f, 0x7f, 0x7f];

/// 读 S3511 的 7 个时间寄存器原始字节（不做校验）。
fn read_regs(link: &mut CartridgeLink) -> [u8; 7] {
    // 使能 GPIO
    gpio_write(link, ADDR_CTRL, 0x0001);
    // 初始状态：全输出，SCK=1, CS=0（空闲）
    gpio_write(link, ADDR_DIR, CS | SCK | SIO);
    gpio_write(link, ADDR_DATA, SCK);

    // 发读命令 0xA6（读全部7寄存器，LSB 先）
    send_byte(link, 0xA6);

    let mut raw = [0u8; 7];
    for (i, slot) in raw.iter_mut().enumerate() {
        *slot = recv_byte(link) & REG_MASKS[i];
    }

    // CS 释放（SCK=1, CS=0）
    gpio_write(link, ADDR_DATA, SCK);
    // 关闭 GPIO
    gpio_write(link, ADDR_CTRL, 0x0000);

    raw
}

/// 校验并解码 7 个寄存器字节；不像真实时钟则返回 None。
///
/// 判据（实测于 4BTP 卡，见 `test/gbtest/gba_rtc_probe.py` 采集的特征）：
/// 1. **7 字节不得全同**。没有 GPIO 的卡上 SIO 是一个固定的 ROM 位，接收例程反复读同一
///    地址 0xC4 的该位，于是每个字节只能是 0x00 或 0xFF 且必然全同 —— 这是「无 RTC」的
///    结构性签名。实卡上该处 ROM 读全零，7 字节确实全是 0x00。
/// 2. 每个字节都是合法 BCD，且各字段在范围内。0x00 会因月/日为 0 越界，0xFF 会因 BCD
///    非法而被挡下。
///
/// 注意不能用「GPIO 数据口回读是否跟随写入值」做判据：实测引脚设为输出时该口恒读 0x00。
fn decode(raw: [u8; 7]) -> Option<RtcTimeGba> {
    if raw.iter().all(|&b| b == raw[0]) {
        return None;
    }
    if !raw.iter().all(|&b| bcd_valid(b)) {
        return None;
    }

    let [year, month, date, dow, hour, minute, second] = raw.map(bcd);
    let ok = (1..=12).contains(&month)
        && (1..=31).contains(&date)
        && dow <= 6
        && hour <= 23
        && minute <= 59
        && second <= 59;
    if !ok {
        return None;
    }

    Some(RtcTimeGba {
        year: year as u16 + 2000,
        month,
        date,
        day_of_week: dow,
        hour,
        minute,
        second,
    })
}

/// 读取 S3511 全部时间寄存器。无 RTC / 读数不合法时返回 None。
pub fn read_s3511(link: &mut CartridgeLink) -> Option<RtcTimeGba> {
    decode(read_regs(link))
}

/// 探测卡上是否真挂着可用的 S3511。
///
/// `info` 用它替代 game code 前缀启发式：启发式只认名单里的几个官方卡号，
/// 自制卡 / 名单外的卡（如 4BTP）会被漏判成无 RTC。
pub fn detect(link: &mut CartridgeLink) -> bool {
    read_s3511(link).is_some()
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn accepts_real_sample_from_4btp_cart() {
        // test/gbtest/gba_rtc_probe.py 在 4BTP 卡上实采：2000-01-23 dow=1 06:18:46
        let t = decode([0x00, 0x01, 0x23, 0x01, 0x06, 0x18, 0x46]).expect("实采读数应判为有效");
        assert_eq!(t.year, 2000);
        assert_eq!((t.month, t.date, t.day_of_week), (1, 23, 1));
        assert_eq!((t.hour, t.minute, t.second), (6, 18, 46));
    }

    #[test]
    fn rejects_all_zero_and_all_ff() {
        // 无 RTC 卡的两种签名：SIO 那位恒 0 或恒 1
        assert!(decode([0x00; 7]).is_none(), "全零应判为无 RTC");
        assert!(decode([0xFF; 7]).is_none(), "全 FF 应判为无 RTC");
    }

    #[test]
    fn rejects_any_constant_byte_pattern() {
        // 总线悬空可能给出别的恒定值，一律按无 RTC 处理
        assert!(decode([0x11; 7]).is_none());
        assert!(decode([0x5A; 7]).is_none());
    }

    #[test]
    fn rejects_out_of_range_fields() {
        let base = [0x00, 0x01, 0x23, 0x01, 0x06, 0x18, 0x46];
        let mut m = base; m[1] = 0x13; // 13 月
        assert!(decode(m).is_none(), "月份 13 应判非法");
        let mut d = base; d[2] = 0x32; // 32 日
        assert!(decode(d).is_none(), "日期 32 应判非法");
        let mut h = base; h[4] = 0x24; // 24 时
        assert!(decode(h).is_none(), "小时 24 应判非法");
        let mut s = base; s[6] = 0x60; // 60 秒
        assert!(decode(s).is_none(), "秒 60 应判非法");
    }

    #[test]
    fn rejects_invalid_bcd_nibbles() {
        let mut r = [0x00, 0x01, 0x23, 0x01, 0x06, 0x18, 0x46];
        r[5] = 0x1A; // 低 nibble = A，非 BCD
        assert!(decode(r).is_none(), "非法 BCD nibble 应被挡下");
    }
}
