# GB/GBC · 存档 RAM 读写

GB / GBC 卡带**电池存档 RAM** 的导出（备份）/ 写入（还原）/ 校验。RAM 同样经 MBC 访问。

## 操作清单

| 操作 | 说明 |
|------|------|
| 导出 RAM | 逐 RAM bank 读出到 `.sav` 文件 |
| 写入 RAM | 把 `.sav` 按 bank 写回卡带 |
| 校验 RAM | 回读与 `.sav` 比对 |

## 对应命令（规划）

```
cfburn gb ram export --port COM7 --out save.sav
cfburn gb ram write  --port COM7 --in  save.sav
cfburn gb ram verify --port COM7 --ref save.sav
```

## 实现状态：🔴 待实现（依赖 MBC 层移植）

### 待办

- [ ] 使能卡带 RAM：向 `0x0000–0x1FFF` 区写 `0x0A`（MBC RAM enable）
- [ ] 切 RAM bank：向 `0x4000–0x5FFF` 区写 bank 号
- [ ] 读写 RAM 窗口 `0xA000–0xBFFF`
- [ ] RAM 大小解析（卡带头 `0x0149`）
- [ ] 可选：MBC3 RTC 读写（原 GUI 有 `Form_mbc3_rtc`，待移植）

## 技术要点

- 强依赖 [GB ROM 子模块](../rom/README.md) 的 mapper 探测与 bank 切换。
- 用完务必**关掉 RAM 使能**（写 `0x00`），否则可能掉存档。
- MBC2 内置 512×4bit RAM，较特殊。
