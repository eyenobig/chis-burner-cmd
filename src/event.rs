//! cfb 的结构化输出事件（NDJSON 契约 v1）。
//!
//! `--json` 模式下，每个子命令把进度/结果打成**一行一个 JSON 对象**（NDJSON）输出到
//! stdout，供 Electron / Tauri 等 JS 客户端逐行 `JSON.parse` 解析展示。
//! 所有事件都带 `type` 字段做判别。新增命令时**先在此登记事件**，再实现，保持格式稳定。
//!
//! 详细契约见仓库 `.claude/skills/cfb-output/SKILL.md`。

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
        // ---- 游戏 ROM 头（仅当识别到 GBA 游戏时非 null）----
        game_name: Option<String>,
        rom_title: Option<String>,
        game_code: Option<String>,
        revision: Option<u8>,
        rom_checksum: Option<RomChecksum>,
        /// 是否带 RTC（GBA 按 game code 启发式判断）。
        rtc: Option<bool>,
    },

    /// 出错（未实现、未知命令、设备异常等）。客户端可据此提示。
    Error { command: String, message: String },

    /// burn/erase/dump：进度（字节）。
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
}

/// 输出一行 NDJSON。
pub fn emit(ev: &Event) {
    if let Ok(s) = serde_json::to_string(ev) {
        println!("{s}");
    }
}
