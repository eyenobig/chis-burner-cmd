# GB/GBC · ROM 读写

GB / GBC 卡带 ROM 的读取 / 烧录 / 擦除 / 导出 / 校验。ROM 通过 MBC **bank 切换**访问。

## 操作清单

| 操作 | 说明 |
|------|------|
| 读取 / 导出 ROM | 逐 bank 切换并回读，拼成完整 ROM dump |
| 烧录 / 写入 ROM | 擦除 → 按 bank 写入 → 校验 |
| 擦除 ROM | 整片 / 按区擦除 |
| 校验 ROM | 逐 bank 回读与源文件比对 |

## 对应命令（规划）

```
cfburn gb rom read   --port COM7 --out dump.gb
cfburn gb rom write  --port COM7 --rom game.gb [--mbc auto|mbc5|mbc3|mbc1]
cfburn gb rom erase  --port COM7
cfburn gb rom verify --port COM7 --ref game.gb
```

## 实现状态：🔴 待实现（依赖 MBC 层移植）

| 操作 | 状态 | 备注 |
|------|------|------|
| 读取/烧录/擦除/校验 | 🔴 | 需先在 `Core` 实现 MBC bank 切换；可复用 `CartLink` 串口层 |

### 待办

- [ ] `Core` 新增 `GbCartridge` / `MbcFlasher`：mapper 探测 + bank 切换
- [ ] ROM 大小解析（卡带头 `0x0148`），按 bank 数遍历
- [ ] 写入路径：根据卡带 Flash 类型选擦写时序

## 技术要点

- ROM bank0 固定在 `0x0000–0x3FFF`，可切换 bank 映射到 `0x4000–0x7FFF`。
- 切 bank：向 `0x2000–0x3FFF` 区写 bank 号（MBC5 还用 `0x3000` 写高位）。
- 校验/导出复用同一套 bank 遍历逻辑。
