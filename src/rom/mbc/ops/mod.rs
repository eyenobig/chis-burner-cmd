//! MBC 操作（实现函数），按 **读 / 写 / 删 / 导** 分文件：
//! `read` · `write` · `delete` · `export` · `rtc` · `save`（GB 总线协议已接通）。

pub mod delete;
pub mod export;
pub mod read;
pub mod rtc;
pub mod save;
pub mod write;
