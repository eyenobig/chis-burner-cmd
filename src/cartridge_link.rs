//! 碳酸丐烧录器的串口协议层（USB CDC，VID 0483 / PID 0721）。
//!
//! 从参考工程 `Z:\Project\beggar_socket\client\ChisFlashBurner.Core\CartLink.cs` 复刻：
//! 命令格式一致，但所有应答读取都带超时；`reconnect()` 在 MCU 卡死后关/重开串口复活。
//! 协议怪癖（必须照搬）：上电/复位（DTR/RTS）后紧跟的第一条命令会被固件吞掉，靠
//! `warm_up()` 吸收。

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use serialport::{ClearBuffer, DataBits, FlowControl, Parity, SerialPort, StopBits};

/// 本烧录器的 USB 标识（STMicroelectronics CDC 虚拟串口）。
pub const USB_VID: u16 = 0x0483;
pub const USB_PID: u16 = 0x0721;

/// 串口波特率（固件固定 9600，USB CDC 下波特率仅占位）。
pub const BAUD: u32 = 9600;

/// 串口协议层。`open()` 后按需 `power_on_3v3()` + `warm_up()`，再发协议命令。
pub struct CartridgeLink {
    port_name: String,
    sp: Option<Box<dyn SerialPort>>,
    /// 单次应答超时。USB 正常应答是毫秒级，超时即认为这一包应答丢失。
    response_timeout: Duration,
}

fn addr4(a: u32) -> [u8; 4] {
    [
        (a & 0xff) as u8,
        ((a >> 8) & 0xff) as u8,
        ((a >> 16) & 0xff) as u8,
        ((a >> 24) & 0xff) as u8,
    ]
}

impl CartridgeLink {
    pub fn new(port: &str) -> Self {
        Self {
            port_name: port.to_string(),
            sp: None,
            response_timeout: Duration::from_millis(800),
        }
    }

    #[allow(dead_code)] // 供后续 burn/校验使用
    pub fn is_open(&self) -> bool {
        self.sp.is_some()
    }

    /// 打开串口并复位命令缓冲。
    pub fn open(&mut self) -> std::io::Result<()> {
        self.close();
        let sp = serialport::new(&self.port_name, BAUD)
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None)
            .timeout(Duration::from_millis(2000))
            .open()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        self.sp = Some(sp);
        self.toggle_reset_lines();
        std::thread::sleep(Duration::from_millis(60));
        self.discard_all();
        Ok(())
    }

    pub fn close(&mut self) {
        self.sp = None; // drop 即关闭
    }

    /// 关/重开串口并重新上电——用于 MCU 卡死后的复活。
    #[allow(dead_code)] // 供后续 burn 的卡死复活使用
    pub fn reconnect(&mut self) -> std::io::Result<()> {
        self.close();
        std::thread::sleep(Duration::from_millis(700));
        self.open()?;
        self.power(1); // 3.3V
        self.warm_up();
        Ok(())
    }

    // DTR/RTS 置位再清零：重置单片机内的命令 buffer。
    fn toggle_reset_lines(&mut self) {
        if let Some(sp) = self.sp.as_mut() {
            let _ = sp.write_request_to_send(true);
            let _ = sp.write_data_terminal_ready(true);
            let _ = sp.write_request_to_send(false);
            let _ = sp.write_data_terminal_ready(false);
        }
    }

    fn discard_all(&mut self) {
        if let Some(sp) = self.sp.as_mut() {
            let _ = sp.clear(ClearBuffer::All);
        }
    }

    /// 底层供电控制（含复位时序）。`code`: 0=断电, 1=3.3V(GBA), 2=5V(GB/GBC)。
    /// 电压**语义/识别**在 `device`（`Voltage`/`power`/`voltage_for`），此处只负责发包。
    pub fn power(&mut self, code: u8) {
        let _ = self.send_package(&[0xa0, code]);
        std::thread::sleep(Duration::from_millis(10));
        self.toggle_reset_lines();
        std::thread::sleep(Duration::from_millis(10));
    }

    /// 吸收上电/复位后“被吞的第一条命令”，并把 flash 复位回读阵列模式。
    pub fn warm_up(&mut self) {
        let save = self.response_timeout;
        self.response_timeout = Duration::from_millis(700);
        for _ in 0..2 {
            self.rom_write(0x00, &[0xf0, 0x00]); // reset to read array
            self.discard_all();
        }
        self.response_timeout = save;
    }

    /// GB 总线版 warm_up：吸收上电/复位后被 MCU 吞掉的第一条命令，并把 flash 复位回读阵列。
    /// MBC 模式 open_powered 后用（C# `mbc5_romGetSize`/`mbc5_romGetID` 末尾都是 gbc_write(0x00, 0xf0)）。
    pub fn gbc_warm_up(&mut self) {
        let save = self.response_timeout;
        self.response_timeout = Duration::from_millis(700);
        for _ in 0..2 {
            self.gbc_write(0x00, &[0xf0]); // reset to read array
            self.discard_all();
        }
        self.response_timeout = save;
    }

    // ---------------- 低层收发 ----------------

    fn send_package(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let size = 2 + payload.len() + 2; // 含末尾 2 字节(忽略的)CRC
        let mut buf = vec![0u8; size];
        buf[0] = (size & 0xff) as u8;
        buf[1] = ((size >> 8) & 0xff) as u8;
        buf[2..2 + payload.len()].copy_from_slice(payload);
        let sp = self
            .sp
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotConnected, "port not open"))?;
        sp.write_all(&buf)
    }

    // 读满 buf.len() 字节，超时(自上次有数据起)返回 false。
    fn read_exact_to(&mut self, buf: &mut [u8]) -> bool {
        let timeout = self.response_timeout;
        let sp = match self.sp.as_mut() {
            Some(s) => s,
            None => return false,
        };
        let n = buf.len();
        let mut got = 0usize;
        let mut last = Instant::now();
        while got < n {
            let avail = sp.bytes_to_read().unwrap_or(0) as usize;
            if avail > 0 {
                let to = avail.min(n - got);
                match sp.read(&mut buf[got..got + to]) {
                    Ok(r) if r > 0 => {
                        got += r;
                        last = Instant::now();
                    }
                    Ok(_) => {}
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(_) => return false,
                }
            } else {
                if last.elapsed() > timeout {
                    return false;
                }
                std::thread::sleep(Duration::from_micros(200));
            }
        }
        true
    }

    // 读 1 字节 ack，期望 0xaa。
    fn read_ack(&mut self) -> bool {
        let mut b = [0u8; 1];
        if !self.read_exact_to(&mut b) {
            return false;
        }
        self.discard_all();
        b[0] == 0xaa
    }

    // 读 n 字节数据（前 2 字节是被忽略的 CRC）。
    fn read_data_bytes(&mut self, out: &mut [u8]) -> bool {
        let mut raw = vec![0u8; out.len() + 2];
        if !self.read_exact_to(&mut raw) {
            return false;
        }
        out.copy_from_slice(&raw[2..]);
        true
    }

    // ---------------- 协议命令 ----------------

    /// rom 直接写(透传)，地址为“字”地址。返回是否收到 ack。
    pub fn rom_write(&mut self, word_addr: u32, data: &[u8]) -> bool {
        let mut pl = vec![0u8; 5 + data.len()];
        pl[0] = 0xf5;
        pl[1..5].copy_from_slice(&addr4(word_addr));
        pl[5..].copy_from_slice(data);
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_ack()
    }

    /// rom 读取，地址为“字节”地址。失败(超时)返回 false。
    pub fn rom_read(&mut self, byte_addr: u32, out: &mut [u8]) -> bool {
        let mut pl = [0u8; 7];
        pl[0] = 0xf6;
        pl[1..5].copy_from_slice(&addr4(byte_addr));
        pl[5] = (out.len() & 0xff) as u8;
        pl[6] = ((out.len() >> 8) & 0xff) as u8;
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_data_bytes(out)
    }

    /// 读 Autoselect ID（8 字节）。
    pub fn rom_read_id(&mut self, id8: &mut [u8]) -> bool {
        if self.send_package(&[0xf0]).is_err() {
            return false;
        }
        self.read_data_bytes(id8)
    }

    /// rom 编程(缓冲写)，地址为“字节”地址（cmd 0xf4）。
    pub fn rom_program(&mut self, byte_addr: u32, data: &[u8], buffer_write_bytes: u16) -> bool {
        let mut pl = vec![0u8; 7 + data.len()];
        pl[0] = 0xf4;
        pl[1..5].copy_from_slice(&addr4(byte_addr));
        pl[5] = (buffer_write_bytes & 0xff) as u8;
        pl[6] = ((buffer_write_bytes >> 8) & 0xff) as u8;
        pl[7..].copy_from_slice(data);
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_ack()
    }

    /// 全片擦除命令（仅下发，不等待完成；cmd 0xf1）。
    pub fn rom_erase_chip(&mut self) -> bool {
        if self.send_package(&[0xf1]).is_err() {
            return false;
        }
        self.read_ack()
    }

    // ---------------- GB/GBC 总线（MBC）----------------

    /// GB 卡总线写（cmd 0xfa）：addr 为 GB 地址空间字节地址。
    pub fn gbc_write(&mut self, addr: u32, data: &[u8]) -> bool {
        let mut pl = vec![0u8; 5 + data.len()];
        pl[0] = 0xfa;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5..].copy_from_slice(data);
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_ack()
    }

    /// GB 卡总线读（cmd 0xfb）。
    pub fn gbc_read(&mut self, addr: u32, out: &mut [u8]) -> bool {
        let mut pl = [0u8; 7];
        pl[0] = 0xfb;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5] = (out.len() & 0xff) as u8;
        pl[6] = ((out.len() >> 8) & 0xff) as u8;
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_data_bytes(out)
    }

    /// GB 卡 ROM 编程(缓冲写)（cmd 0xfc）。
    pub fn gbc_rom_program(&mut self, addr: u32, data: &[u8], buffer_write_bytes: u16) -> bool {
        let mut pl = vec![0u8; 7 + data.len()];
        pl[0] = 0xfc;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5] = (buffer_write_bytes & 0xff) as u8;
        pl[6] = ((buffer_write_bytes >> 8) & 0xff) as u8;
        pl[7..].copy_from_slice(data);
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_ack()
    }

    // ---------------- 存档 RAM（GBA 侧 0xE0000000 区）----------------
    // 复刻自 `cart_adapter.cs` 的 ram_*：地址均为 GBA 字节地址。

    /// SRAM 写（cmd 0xf7）。addr 为 GBA 字节地址。
    pub fn ram_write(&mut self, addr: u32, data: &[u8]) -> bool {
        let mut pl = vec![0u8; 5 + data.len()];
        pl[0] = 0xf7;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5..].copy_from_slice(data);
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_ack()
    }

    /// SRAM 读（cmd 0xf8）。addr 为 GBA 字节地址。
    pub fn ram_read(&mut self, addr: u32, out: &mut [u8]) -> bool {
        let mut pl = [0u8; 7];
        pl[0] = 0xf8;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5] = (out.len() & 0xff) as u8;
        pl[6] = ((out.len() >> 8) & 0xff) as u8;
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_data_bytes(out)
    }

    /// FLASH 存档编程（cmd 0xf9）。addr 为 GBA 字节地址。
    pub fn ram_flash_program(&mut self, addr: u32, data: &[u8]) -> bool {
        let mut pl = vec![0u8; 5 + data.len()];
        pl[0] = 0xf9;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5..].copy_from_slice(data);
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_ack()
    }

    /// FRAM 存档写（cmd 0xe7）。布局：cmd + addr4 + latency + data（latency 在数据前）。
    pub fn ram_write_fram(&mut self, addr: u32, data: &[u8], latency: u8) -> bool {
        let mut pl = vec![0u8; 6 + data.len()];
        pl[0] = 0xe7;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5] = latency;
        pl[6..].copy_from_slice(data);
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_ack()
    }

    /// FRAM 存档读（cmd 0xe8）。布局：cmd + addr4 + len2 + latency（latency 在末尾）。
    pub fn ram_read_fram(&mut self, addr: u32, out: &mut [u8], latency: u8) -> bool {
        let mut pl = [0u8; 8];
        pl[0] = 0xe8;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5] = (out.len() & 0xff) as u8;
        pl[6] = ((out.len() >> 8) & 0xff) as u8;
        pl[7] = latency;
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_data_bytes(out)
    }

    /// GB 总线 FRAM 存档写（cmd 0xea）。latency 在数据前（GBA FRAM 用 25，MBC FRAM 用 10）。
    pub fn gbc_write_fram(&mut self, addr: u32, data: &[u8], latency: u8) -> bool {
        let mut pl = vec![0u8; 6 + data.len()];
        pl[0] = 0xea;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5] = latency;
        pl[6..].copy_from_slice(data);
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_ack()
    }

    /// GB 总线 FRAM 存档读（cmd 0xeb）。latency 在末尾。
    pub fn gbc_read_fram(&mut self, addr: u32, out: &mut [u8], latency: u8) -> bool {
        let mut pl = [0u8; 8];
        pl[0] = 0xeb;
        pl[1..5].copy_from_slice(&addr4(addr));
        pl[5] = (out.len() & 0xff) as u8;
        pl[6] = ((out.len() >> 8) & 0xff) as u8;
        pl[7] = latency;
        if self.send_package(&pl).is_err() {
            return false;
        }
        self.read_data_bytes(out)
    }
}

impl Drop for CartridgeLink {
    fn drop(&mut self) {
        self.close();
    }
}
