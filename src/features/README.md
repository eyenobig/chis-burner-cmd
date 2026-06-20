# 功能模块总览

cfburn 按功能拆成 5 个模块，每个一个目录 + README。当前底层引擎在 [../Core](../Core)，
命令行前端在 [../Cli](../Cli)。

| # | 功能 | 目录 | 状态 |
|---|------|------|------|
| 1 | 识别烧录器 | [device_detect](device_detect/README.md) | ✅ 已实现 |
| 2 | GBA 卡带 读/烧/擦 ROM | [gba_cartridge](gba_cartridge/README.md) | 🟡 部分（烧/擦已实现，读待封装）|
| 3 | GB/GBC 卡带 读/烧/擦 ROM | [gb_cartridge](gb_cartridge/README.md) | 🔴 待移植（MBC 层）|
| 4 | ROM 通用操作 写/导出/校验 | [rom_ops](rom_ops/README.md) | 🟡 部分（原语就绪，缺独立命令）|
| 5 | 存档 RAM 写/导出/校验 | [ram_ops](ram_ops/README.md) | 🔴 待实现 |

## 依赖关系

```
device_detect (连接层 CartLink)
   ├─ gba_cartridge ─┐
   ├─ gb_cartridge ──┼─ 复用 ─> rom_ops (ROM 写/导出/校验原语)
   └─ (各卡带)        └─ 复用 ─> ram_ops (存档读写)
```

- 功能 1 是所有功能的前置（先连上、读信息）。
- 功能 2 / 3 是按平台的卡带流程，建立在功能 4（ROM 原语）和功能 5（RAM）之上。
- 状态图例：✅ 已实现　🟡 部分实现　🔴 待实现。
