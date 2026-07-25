//! GBA · 导：导出（dump ROM 到文件）。
//!
//! 基于 `CartridgeLink::rom_read` 按 4096B 连续读出 `len` 字节写入文件。长度通常取 CFI 容量。

use std::fs::File;
use std::io::Write;

use crate::cartridge_link::CartridgeLink;

const PACKET: usize = 4096;

/// 导出 `len` 字节 ROM 到 `path`。
pub fn dump(
    link: &mut CartridgeLink,
    len: u64,
    path: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> std::io::Result<()> {
    let mut f = File::create(path)?;
    let mut pos = 0u64;
    let mut buf = vec![0u8; PACKET];
    while pos < len {
        let n = ((len - pos) as usize).min(PACKET);
        let b = &mut buf[..n];
        if !link.rom_read(pos as u32, b) {
            let _ = link.reconnect();
            continue;
        }
        f.write_all(b)?;
        pos += n as u64;
        progress(pos, len);
    }
    f.flush()
}
