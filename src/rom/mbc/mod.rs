//! MBC (GB/GBC) ROM：数据集(data，含 maptype) + 实现函数(ops)。
//!
//! 解析逻辑就绪；live 读取待移植 GB 总线协议（`cart_adapter.cs`），暂未接入 `cfb info`。

pub mod data;
pub mod ops;
