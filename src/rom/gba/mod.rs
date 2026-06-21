//! GBA ROM：数据集(data) + 实现函数(ops)。
//!
//! 已实现：flash ID + CFI 容量、卡带在否、GBA 判别、ROM 头解析。
//! 待移植：擦除 / PPB / 健壮编程 / 校验（见参考源 `GbaFlasher.cs` + `mission_gba.cs`）。

pub mod data;
pub mod ops;

pub use data::{FlashInfo, GbaHeader};
