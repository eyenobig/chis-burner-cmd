//! MBC 操作（实现函数），按 **读 / 写 / 删 / 导** 分文件：
//! `read`(解析就绪) · `write` · `delete` · `export`（均待移植 GB 总线协议）。

pub mod delete;
pub mod export;
pub mod read;
pub mod rtc;
pub mod save;
pub mod write;
