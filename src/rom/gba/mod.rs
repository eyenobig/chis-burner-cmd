//! GBA ROM：数据集(data) + 实现函数(ops)。
//!
//! 已实现：flash ID + CFI、卡带在否、GBA 判别、ROM 头解析、擦除 / PPB、
//! 编程烧录与校验、dump、RTC 读取、存档 dump/write/verify。
//! 参考源：`GbaFlasher.cs` + `mission_gba.cs`。

pub mod data;
pub mod ops;

pub use data::{FlashInfo, GbaHeader};
