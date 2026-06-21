//! 设备模块：数据类型（`data` 数据集）+ 操作函数（`ops` 实现函数）。
//!
//! 对外用法不变：`device::cmd_detect` / `cmd_select` / `resolve_port` 等，
//! 由下面的 `pub use` 从 `ops` 重导出。

pub mod data;
pub mod ops;

// 命令入口给 main 用；其余函数/类型在模块内部经 `ops`/`data` 路径直接引用。
pub use data::Voltage;
pub use ops::{cmd_detect, cmd_select, cmd_voltage, power, power_off, resolve_port, voltage_for};
