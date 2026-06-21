//! 通用（跨平台）卡带逻辑：类型枚举(data) + 通用判断/解析工具(ops)。

pub mod data;
pub mod ops;

pub use data::CartridgeKind;
