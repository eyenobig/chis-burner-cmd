//! 识别诊断上下文：只在 info 命令作用域内启用，不记录 ROM/存档内容。
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::event::{emit, Event};

struct Context {
    json: bool,
    trace_id: String,
    port: Option<String>,
    platform: &'static str,
    attempt: u32,
    started: Instant,
    stage_started: Instant,
    stage: String,
    stage_starts: HashMap<String, Instant>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub struct Session;

pub fn enabled() -> bool {
    ACTIVE.with(|active| active.borrow().is_some())
}

impl Drop for Session {
    fn drop(&mut self) {
        ACTIVE.with(|active| *active.borrow_mut() = None);
    }
}

/// 每次进程调用有独立标识；同一次调用的重试共用标识。
pub fn begin(json: bool, port: Option<String>, mbc: bool) -> Session {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    ACTIVE.with(|active| {
        *active.borrow_mut() = Some(Context {
            json,
            trace_id: format!("{}-{timestamp}", std::process::id()),
            port,
            platform: if mbc { "gb_mbc" } else { "gba" },
            attempt: 0,
            started: Instant::now(),
            stage_started: Instant::now(),
            stage: "process".into(),
            stage_starts: HashMap::new(),
        });
    });
    stage(
        "process",
        "start",
        &format!("cfb {}", env!("CARGO_PKG_VERSION")),
    );
    Session
}

pub fn attempt(number: u32) {
    ACTIVE.with(|active| {
        if let Some(context) = active.borrow_mut().as_mut() {
            context.attempt = number;
        }
    });
}

pub fn port(name: &str) {
    ACTIVE.with(|active| {
        if let Some(context) = active.borrow_mut().as_mut() {
            context.port = Some(name.into());
        }
    });
}

/// start 写在阻塞调用之前并立即 flush；进程被杀时仍能找到最后进入的阶段。
pub fn stage(name: &str, status: &str, message: &str) {
    ACTIVE.with(|active| {
        if let Some(context) = active.borrow_mut().as_mut() {
            if status == "start" {
                context.stage = name.into();
                context.stage_started = Instant::now();
                context
                    .stage_starts
                    .insert(name.into(), context.stage_started);
            }
            output(context, name, status, message);
        }
    });
}

/// 底层失败沿用当前阶段，避免将串口通信错误误报为卡带不在位。
pub fn failure(operation: &str, message: &str) {
    ACTIVE.with(|active| {
        if let Some(context) = active.borrow().as_ref() {
            output(
                context,
                &context.stage,
                "failed",
                &format!("{operation}: {message}"),
            );
        }
    });
}

fn event(context: &Context, stage: &str, status: &str, message: &str) -> Event {
    Event::Diagnostic {
        command: "info".into(),
        trace_id: context.trace_id.clone(),
        pid: std::process::id(),
        port: context.port.clone(),
        platform: context.platform.into(),
        attempt: context.attempt,
        stage: stage.into(),
        status: status.into(),
        elapsed_ms: context.started.elapsed().as_millis() as u64,
        stage_elapsed_ms: context
            .stage_starts
            .get(stage)
            .unwrap_or(&context.stage_started)
            .elapsed()
            .as_millis() as u64,
        message: message.into(),
    }
}

fn output(context: &Context, stage: &str, status: &str, message: &str) {
    let record = event(context, stage, status, message);
    if context.json {
        emit(&record);
    } else if let Ok(line) = serde_json::to_string(&record) {
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_preserves_failure_context_and_excludes_payload() {
        let context = Context {
            json: true,
            trace_id: "trace-1".into(),
            port: Some("COM3".into()),
            platform: "gba",
            attempt: 2,
            started: Instant::now(),
            stage_started: Instant::now(),
            stage: "rtc".into(),
            stage_starts: HashMap::new(),
        };
        let value = serde_json::to_value(event(
            &context,
            "rtc",
            "failed",
            "read: expected=4 received=0",
        ))
        .unwrap();
        assert_eq!(value["type"], "diagnostic");
        assert_eq!(value["trace_id"], "trace-1");
        assert_eq!(value["port"], "COM3");
        assert_eq!(value["attempt"], 2);
        assert_eq!(value["stage"], "rtc");
        assert_eq!(value["status"], "failed");
        assert!(value["stage_elapsed_ms"].is_u64());
        assert!(value.get("payload").is_none(), "诊断不得含卡带数据");
    }

    #[test]
    fn session_clears_context_on_early_return() {
        {
            let _session = begin(true, None, false);
        }
        ACTIVE.with(|active| assert!(active.borrow().is_none(), "命令退出后必须关闭诊断"));
    }
}
