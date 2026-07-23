//! GBA 操作（实现函数），按 **读 / 写 / 删 / 导** 分文件：
//! `read`(已实现) · `write`(待移植) · `delete`(待移植) · `export`(待实现)。

pub mod delete;
pub mod export;
pub mod read;
pub mod rtc;
pub mod save;
pub mod write;

// 读侧函数对外用 `gba::ops::read_info` 等扁平路径访问。
pub use read::*;
