//! 统一进度展示：`擦除 45% · 12.3s` / `写入 78% · 45.1s`。
//!
//! 擦除、写入、校验、读取共用同一格式；[`ProgressLog`] 负责节流，避免逐包刷屏，
//! 同时在百分比前进或长时间停滞时持续输出已用时间。

use std::time::{Duration, Instant};

use crate::i18n;

/// 进度阶段（决定标签文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Dump 供 dump/读取路径复用同一格式
pub enum Phase {
    Erase,
    Write,
    Verify,
    Dump,
}

impl Phase {
    pub fn label(self) -> String {
        match self {
            Phase::Erase => i18n::t("progress.label.erase"),
            Phase::Write => i18n::t("progress.label.write"),
            Phase::Verify => i18n::t("progress.label.verify"),
            Phase::Dump => i18n::t("progress.label.dump"),
        }
    }
}

/// `done/total` → 0..=100 的整数百分比。
pub fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    let pct = ((done as f64 / total as f64) * 100.0).floor() as u32;
    pct.min(100)
}

/// 格式化一行进度：`{label} {pct}% · {s}s`（中文优先，走 i18n）。
pub fn format_progress(label: &str, done: u64, total: u64, elapsed_secs: f64) -> String {
    let pct = percent(done, total);
    format_progress_pct(label, pct, elapsed_secs)
}

/// 已知百分比时的格式化（整片擦按耗时估算进度时用）。
pub fn format_progress_pct(label: &str, pct: u32, elapsed_secs: f64) -> String {
    let pct = pct.min(100);
    i18n::tf(
        "progress.line",
        &[
            ("label", label),
            ("pct", &pct.to_string()),
            ("s", &format!("{elapsed_secs:.1}")),
        ],
    )
}

/// 节流进度日志：百分比前进、起止、或停滞超时才输出。
pub struct ProgressLog {
    phase: Phase,
    start: Instant,
    last_pct: Option<u32>,
    last_log: Instant,
    /// 百分比连跳时的最小间隔，避免瞬时刷屏。
    min_interval: Duration,
    /// 同百分比长时间无进展时的心跳间隔（刷新已用时间）。
    stall_interval: Duration,
}

impl ProgressLog {
    pub fn new(phase: Phase) -> Self {
        let now = Instant::now();
        Self {
            phase,
            start: now,
            last_pct: None,
            last_log: now.checked_sub(Duration::from_secs(60)).unwrap_or(now),
            min_interval: Duration::from_millis(400),
            stall_interval: Duration::from_secs(5),
        }
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn format(&self, done: u64, total: u64) -> String {
        format_progress(&self.phase.label(), done, total, self.elapsed_secs())
    }

    /// 是否应打一行进度 log；调用后若返回 true 会更新内部节流状态。
    pub fn should_log(&mut self, done: u64, total: u64) -> bool {
        let pct = percent(done, total);
        let now = Instant::now();
        let since = now.saturating_duration_since(self.last_log);
        let at_start = done == 0 && self.last_pct.is_none();
        let at_end = total > 0 && done >= total && self.last_pct != Some(100);
        let force = at_start || at_end;
        let pct_bump = self.last_pct.map(|p| pct > p).unwrap_or(true);
        let emit = force
            || (pct_bump && since >= self.min_interval)
            || since >= self.stall_interval;
        if emit {
            self.last_pct = Some(pct);
            self.last_log = now;
        }
        emit
    }

    /// `progress(done,total)` + 节流 `log(统一格式)`。
    pub fn report(
        &mut self,
        done: u64,
        total: u64,
        progress: &mut dyn FnMut(u64, u64),
        log: &mut dyn FnMut(&str),
    ) {
        progress(done, total);
        if self.should_log(done, total) {
            log(&self.format(done, total));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_basic() {
        assert_eq!(percent(0, 100), 0);
        assert_eq!(percent(45, 100), 45);
        assert_eq!(percent(100, 100), 100);
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(0, 0), 0);
    }

    #[test]
    fn format_contains_pct_and_time() {
        // 未 init i18n 时回退 key 或 zh-CN；这里直接测底层拼装逻辑
        let s = format_progress_pct("擦除", 45, 12.34);
        // i18n 未 init 时可能返回模板或译文；至少应含 45% 与时间数字
        assert!(s.contains("45%"), "{s}");
        assert!(s.contains("12.3"), "{s}");
    }

    #[test]
    fn throttle_skips_same_pct_until_stall() {
        let mut pl = ProgressLog::new(Phase::Write);
        pl.min_interval = Duration::from_millis(0);
        pl.stall_interval = Duration::from_secs(60);
        assert!(pl.should_log(0, 100)); // 起点
        assert!(pl.should_log(10, 100)); // 10%
        assert!(!pl.should_log(10, 100)); // 同百分比
        assert!(pl.should_log(11, 100)); // 前进
        assert!(pl.should_log(100, 100)); // 终点
    }
}
