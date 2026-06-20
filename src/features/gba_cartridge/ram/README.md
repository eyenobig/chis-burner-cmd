# GBA · 存档 RAM 读写

GBA 卡带**存档**的导出（备份）/ 写入（还原）/ 校验。

## 操作清单

| 操作 | 说明 |
|------|------|
| 导出 RAM | 把存档读出到 `.sav` 文件 |
| 写入 RAM | 把 `.sav` 写回卡带 |
| 校验 RAM | 回读与 `.sav` 比对 |

## 对应命令（规划）

```
cfburn gba ram export --port COM7 --out save.sav
cfburn gba ram write  --port COM7 --in  save.sav
cfburn gba ram verify --port COM7 --ref save.sav
```

## 实现状态：🔴 待实现

当前 `Core` 只有 ROM/Flash 命令，**没有存档 RAM 通道**，需新增。

### 待办

- [ ] `Core` 新增存档读写命令（独立于 ROM 地址空间）
- [ ] 存档类型探测与处理：**SRAM / Flash(512K·1M) / EEPROM(4K·64K)**，三类时序不同，不能一套通吃
- [ ] 可选：GBA RTC 读写（原 GUI 有 `Form_gba_rtc`，待移植）

## 技术要点

- 三类存档识别方式不同（常按游戏库或试探），写时序差异大。
- EEPROM/Flash 的擦写时序最易踩坑；SRAM 最简单（直接读写）。
- 存档体积小（≤128KB 量级）。
