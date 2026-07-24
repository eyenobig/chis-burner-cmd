//! cfb 的结构化输出事件（NDJSON 契约 v1）。
//!
//! `--json` 模式下，每个子命令把进度/结果打成**一行一个 JSON 对象**（NDJSON）输出到
//! stdout，供 Electron / Tauri 等 JS 客户端逐行 `JSON.parse` 解析展示。
//! 所有事件都带 `type` 字段做判别。新增命令时**先在此登记事件**，再实现，保持格式稳定。
//!
//! 详细契约见仓库 `docs/client-protocol.md`。

use serde::Serialize;

/// ROM 头校验：固件里存的值、按规范算出的值、是否一致。
#[derive(Serialize, Clone)]
pub struct RomChecksum {
    pub stored: u8,
    pub computed: u8,
    pub ok: bool,
}

/// 一个 NDJSON 事件。内部用 `type` 字段标注种类。
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// detect/select：一个烧录器（detect 只输出烧录器，不含其他串口）。
    Port {
        port: String,
        /// 4 位大写十六进制，如 "0483"。
        vid: Option<String>,
        pid: Option<String>,
        /// 恒为 true（detect 已过滤为仅烧录器）；保留字段便于客户端统一处理。
        burner: bool,
        /// 当前是否可打开（false=被占用/不可用）。
        open: bool,
        name: String,
        /// USB 序列号；读不到时为 null（兼容字段，客户端可忽略）。
        #[serde(skip_serializing_if = "Option::is_none")]
        serial: Option<String>,
    },

    /// 命令结束时的汇总（detect 只统计烧录器数量）。
    Summary { command: String, burners: usize },

    /// select 结果：记住的端口；`port` 为 null 表示已清除。
    Selected { port: Option<String> },

    /// info：flash 芯片 + 卡带/游戏信息。
    Info {
        port: String,
        /// 是否检测到卡带（flash 芯片有有效响应）。
        present: bool,
        /// 卡带类型："gba" / "gb_mbc" / "unknown"。
        kind: String,
        // ---- flash 芯片（CFI）----
        /// Autoselect ID，"01 02 ..." 形式。
        id: String,
        /// 容量（字节）。
        capacity_bytes: u64,
        /// 写缓冲大小（字节），0=仅单字编程。
        buffer_write_bytes: u32,
        sector_size: u32,
        sector_count: u32,
        // ---- 游戏 ROM 头（识别到 GBA 或 GB/GBC(MBC) 游戏时非 null）----
        game_name: Option<String>,
        rom_title: Option<String>,
        game_code: Option<String>,
        revision: Option<u8>,
        rom_checksum: Option<RomChecksum>,
        /// 是否带 RTC（GBA 按 game code 启发式判断）。
        rtc: Option<bool>,
        /// 存档 RAM 大小（字节）。MBC 从头 0x149 解析；GBA 无法直接得知（需探测），为 None。
        save_size_bytes: Option<u64>,
        /// GB/GBC(MBC) 专属：卡带类型原始字节（头 0x147）。GBA 恒为 null（GBA 无此概念，用 game_code 代替）。
        cartridge_type: Option<u8>,
        /// GB/GBC(MBC) 专属：卡带类型对应的 MBC 名称（如 "MBC5"）。GBA 恒为 null。
        mbc_name: Option<String>,
        /// GB/GBC 免电存档（`db_DMG_bl`）：ROM 内偏移；未命中为 null。
        batteryless_offset: Option<u64>,
        /// 免电逻辑存档字节数。
        batteryless_size: Option<u64>,
        /// 免电布局：0=连续，1=bank 前半，2=bank 后半。
        batteryless_layout: Option<u8>,
    },

    /// 出错（未实现、未知命令、设备异常等）。客户端可据此提示。
    Error { command: String, message: String },

    /// burn/erase/dump：进度。
    /// - 写入/导出：`done`/`total` 为字节。
    /// - 扇区擦除（`erase_range_logged`）：`done`/`total` 为扇区数（含开局 `0/total`）。
    Progress { done: u64, total: u64 },

    /// burn/erase/dump：阶段性日志。
    Log { message: String },

    /// burn/erase/dump：最终结果。
    Result {
        command: String,
        ok: bool,
        bytes: u64,
        mismatch_bytes: u32,
        seconds: f64,
    },

    /// voltage：当前/设置的供电电压（"3.3V"/"5V"/"off"/"auto"）。
    Voltage { voltage: String },

    /// version：`cfb version` 报告的版本号（来自 Cargo.toml）。
    Version { version: String },

    /// rtc read：从卡带读到的 RTC 时间。
    RtcData {
        ok: bool,
        /// "gba"（S3511 GPIO bit-bang）或 "mbc3"（内存映射寄存器）。
        kind: String,
        // GBA S3511 字段（BCD 已转十进制）
        year: Option<u16>,
        month: Option<u8>,
        date: Option<u8>,
        day_of_week: Option<u8>,
        // 公共字段
        hour: Option<u8>,
        minute: Option<u8>,
        second: Option<u8>,
        // MBC3 专用
        day_count: Option<u16>,
        halted: Option<bool>,
        overflow: Option<bool>,
    },

    /// save-dump/write/verify：定位到的存档信息。
    /// `offset` 仅免电存档有值（存档藏在 ROM flash 内的绝对偏移）；其余为 null。
    SaveInfo {
        /// 存档类型："SRAM" / "FLASH" / "FRAM" / "Batteryless"。
        save_type: String,
        offset: Option<u64>,
        /// 存档字节数。
        size: u64,
    },
}

/// 输出一行 NDJSON（立即 flush，保证长耗时扇区擦除期间进度能实时到达客户端）。
pub fn emit(ev: &Event) {
    use std::io::Write;
    if let Ok(s) = serde_json::to_string(ev) {
        println!("{s}");
        let _ = std::io::stdout().flush();
    }
}
