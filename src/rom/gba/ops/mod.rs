//! GBA 操作（实现函数），按 **读 / 写 / 删 / 导** 分文件：
//! `read` · `write` · `delete` · `export` · `rtc` · `save`（均已实现）。

pub mod delete;
pub mod export;
pub mod read;
pub mod rtc;
pub mod save;
pub mod write;

// 读侧函数对外用 `gba::ops::read_info` 等扁平路径访问。
pub use read::*;
