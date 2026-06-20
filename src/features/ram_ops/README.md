# 功能 5 · 存档 RAM 操作

卡带**存档 RAM**（玩家存档）的 **写入 / 导出 / 校验**。即"备份存档 / 还原存档 / 校验存档"。

## 操作清单

| 操作 | 说明 |
|------|------|
| 导出 RAM | 把卡带存档读出到 `.sav` 文件（备份） |
| 写入 RAM | 把 `.sav` 写回卡带（还原） |
| 校验 RAM | 回读并与 `.sav` 比对 |

## 对应命令（规划）

```
cfburn ram export --port COM7 --out save.sav
cfburn ram write  --port COM7 --in  save.sav
cfburn ram verify --port COM7 --ref save.sav
```

## 实现状态：🔴 待实现

当前 `Core`（`CartLink` / `GbaFlasher`）只有 **ROM/Flash** 命令，**没有任何存档 RAM 通道**。需新增。

### 待办

- [ ] `Core` 新增 RAM 读写命令（区别于 ROM 地址空间）
- [ ] GBA 存档类型探测与处理：**SRAM / Flash(512K·1M) / EEPROM(4K·64K)** 各自时序不同
- [ ] GB/GBC 电池 SRAM：需先用 MBC 使能 RAM（写 `0x0000` 区 `0x0A`）、切 RAM bank
- [ ] 可选：MBC3 / GBA 的 **RTC**（实时钟）读写（原 GUI 有 `Form_gba_rtc` / `Form_mbc3_rtc`，待移植）

## 技术要点

- GBA 存档四类，识别方式不同（常按游戏库或试探），写时序差异大，**不能一套逻辑通吃**。
- GB 电池 RAM 读写依赖 MBC，强依赖 [功能 3 · gb_cartridge](../gb_cartridge/README.md) 的 mapper 层。
- 存档体积小（≤128KB 量级），但 EEPROM/Flash 的擦写时序最易踩坑。
