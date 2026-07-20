//! cfb —— 碳酸丐烧录器命令行 (Rust 版)。
//!
//!   cfb [--lang L] <命令> [选项]
//!     detect [--json]                列出已连接的烧录器
//!     select [--port P] [--clear]    选择并记住一个烧录器
//!     voltage [3v3|5v|off|auto]      记住/查看供电电压偏好
//!     disconnect                     断开烧录器并清除记住的端口
//!     info  [--port P] [--mbc]       读 flash + 卡带/游戏信息
//!     rom-info --file <f>            离线解析 ROM 文件头
//!     burn  --rom <f> [--mbc] [...]  写入 ROM
//!     erase [--mbc]                  清空 ROM（整片擦除）
//!     dump  --out <f> [--mbc] [--len N]  导出 ROM 到文件
//!     rtc   [--mbc]                  读取卡带 RTC
//!     help
//!
//! 全局：`--lang zh-CN|en`（会被记住）；`--json` 输出 NDJSON 事件流。
//! 偏好（端口/语言/电压）持久化在 ~/.cfb.json。

mod cartridge_link;
mod config;
mod device;
mod event;
mod i18n;
mod rom;

use std::process::ExitCode;

use event::{emit, Event};

/// 取值型选项名（解析 positional 时跳过它们的值）。
const VALUE_OPTS: &[&str] = &["--lang", "--port", "--rom", "--out", "--len", "--file"];

fn opt_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// 非选项参数（位置参数）：positionals[0]=命令，[1]=子参数（如 voltage 的值）。
fn positionals(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if VALUE_OPTS.contains(&a) {
            i += 2;
        } else if a.starts_with('-') {
            i += 1;
        } else {
            out.push(a);
            i += 1;
        }
    }
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // 语言：--lang 优先（并记住），否则用记住的，再否则跟随系统。
    let lang_arg = opt_value(&args, "--lang");
    if let Some(l) = &lang_arg {
        config::save_lang(l);
    }
    i18n::init(lang_arg.or_else(config::load_lang).as_deref());

    let json = has_flag(&args, "--json");
    let port = opt_value(&args, "--port");
    let clear = has_flag(&args, "--clear");
    let mbc = has_flag(&args, "--mbc");

    let pos = positionals(&args);
    let cmd = pos.first().copied().unwrap_or("");

    match cmd {
        "detect" | "devices" => device::cmd_detect(json),
        "select" => device::cmd_select(json, port, clear),
        "disconnect" => device::cmd_disconnect(json, port),
        "voltage" => device::cmd_voltage(json, pos.get(1).map(|s| s.to_string()), clear),
        "info" => rom::cmd_info(json, port, mbc),
        "rom-info" => match opt_value(&args, "--file") {
            Some(f) => rom::cmd_rom_info(json, &f),
            None => arg_required(json, "rom-info", "op.no_rom"),
        },
        "burn" | "write" => match opt_value(&args, "--rom") {
            Some(f) => rom::cmd_burn(
                json,
                port,
                &f,
                mbc,
                has_flag(&args, "--chip-erase"),
                !has_flag(&args, "--no-ppb"),
                !has_flag(&args, "--no-verify"),
            ),
            None => arg_required(json, "burn", "op.no_rom"),
        },
        "erase" => rom::cmd_erase(json, port, mbc),
        "rtc" => rom::cmd_rtc_read(json, port, mbc),
        "dump" => match opt_value(&args, "--out") {
            Some(f) => {
                let len = opt_value(&args, "--len").and_then(|s| s.parse::<u64>().ok());
                rom::cmd_dump(json, port, &f, mbc, len)
            }
            None => arg_required(json, "dump", "op.no_out"),
        },
        "" | "help" | "-h" | "--help" => {
            println!("{}", i18n::t("usage"));
            ExitCode::SUCCESS
        }
        other => {
            let msg = i18n::tf("err.unknown_cmd", &[("cmd", other)]);
            if json {
                emit(&Event::Error { command: other.to_string(), message: msg });
            } else {
                eprintln!("{msg}\n");
                println!("{}", i18n::t("usage"));
            }
            ExitCode::from(2)
        }
    }
}

fn arg_required(json: bool, cmd: &str, key: &str) -> ExitCode {
    let msg = i18n::t(key);
    if json {
        emit(&Event::Error { command: cmd.to_string(), message: msg });
    } else {
        eprintln!("{msg}");
    }
    ExitCode::from(2)
}
