//! MBC (GB/GBC) ROM：数据集(data，含 maptype) + 实现函数(ops)。
//!
//! 头解析与 live 读取均通过 `ops::read` 提供。

pub mod data;
pub mod ops;
