# 功能模块总览

cfburn 按功能拆成模块，每个一个目录 + README。底层引擎在 [../Core](../Core)，命令行前端在 [../Cli](../Cli)。

ROM / RAM 操作在 GBA 与 GB 上实现方式不同（GBA 线性 NOR Flash；GB 走 MBC bank 切换），
因此**归到各自平台内部**，不再平铺。

```
features/
├─ device_detect/          功能1 识别烧录器
├─ gba_cartridge/          功能2 GBA 卡带
│  ├─ rom/                   读取/烧录/擦除/导出/校验 ROM
│  └─ ram/                   写入/导出/校验 存档
└─ gb_cartridge/           功能3 GB/GBC 卡带
   ├─ rom/                   读取/烧录/擦除/导出/校验 ROM
   └─ ram/                   写入/导出/校验 存档
```

| 功能 | 子模块 | 状态 |
|------|--------|------|
| [1 识别烧录器](device_detect/README.md) | — | ✅ 已实现 |
| [2 GBA 卡带](gba_cartridge/README.md) | [rom](gba_cartridge/rom/README.md) | 🟡 部分（烧/擦已实现，读/校验待封装）|
| | [ram](gba_cartridge/ram/README.md) | 🔴 待实现 |
| [3 GB/GBC 卡带](gb_cartridge/README.md) | [rom](gb_cartridge/rom/README.md) | 🔴 待移植（MBC 层）|
| | [ram](gb_cartridge/ram/README.md) | 🔴 待移植（MBC 层）|

## 依赖关系

```
device_detect (连接层 CartLink，平台无关)
   ├─ gba_cartridge ─ rom / ram   (线性 NOR Flash + SRAM/Flash/EEPROM)
   └─ gb_cartridge  ─ rom / ram   (MBC bank 切换 + 电池 RAM)
```

- 功能 1 是所有功能的前置（先连上、读信息）。
- 功能 2 / 3 各自把 ROM 与存档 RAM 作为两个独立地址通道处理。
- 状态图例：✅ 已实现　🟡 部分实现　🔴 待实现。
