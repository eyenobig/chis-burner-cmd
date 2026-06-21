# 硬件测试清单（一项一项过）

写/清/导 的引擎都是从参考源 `Z:\Project\beggar_socket\client` 复刻的，**尚未经过真机测试**。
插上烧录器（VID 0483 / PID 0721）和对应卡带后，按下面顺序逐项验证，过一项打一个勾。

> 二进制：`cargo build --release` → `target/release/cfb`（或 `cargo run -- <命令>`）。
> 每条都可加 `--json` 看结构化事件。**烧录会擦除卡带，先用可丢弃的测试卡。**

## 0. 前置

- [ ] `cfb detect` 能列出烧录器（COMx / 0483:0721）
- [ ] `cfb select` 记住烧录器；`~/.cfb.json` 出现 `"port"`
- [ ] `cfb info` 读到 flash ID + 容量（插 GBA 卡时还应有标题/GameCode 等）

## 1. 偏好持久化（记录功能）

- [ ] `cfb --lang en detect` 后，再跑 `cfb detect` 仍是英文（语言被记住）
- [ ] `cfb --lang zh-CN detect` 切回中文
- [ ] `cfb voltage 5v` → `cfb voltage` 显示 5V；`~/.cfb.json` 出现 `"voltage":"5V"`
- [ ] `cfb voltage auto` 清除偏好，回到按卡型自动

## 2. GBA（3.3V）

- [ ] `cfb info` 显示 `卡带: 已检测到 (GBA)` 与正确容量
- [ ] `cfb dump --out gba_backup.gba` 导出整片，文件大小 == 容量；与已知 ROM 比对一致
- [ ] `cfb erase` 整片擦除；之后 `cfb dump` 出来应全 `0xFF`
- [ ] `cfb burn --rom <小测试.gba>` 烧录；结束报 `成功 ✅`，无 mismatch
- [ ] 烧录后 `cfb info` 显示新游戏的标题/GameCode
- [ ] `cfb burn --rom <f> --chip-erase` 整片擦除模式也能烧成功
- [ ] 大 ROM（≥16MB）烧录：进度推进正常，无卡死（验证重连逻辑）

## 3. MBC / GB·GBC（5V）

> GB 卡读写走 5V。先 `cfb voltage 5v` 或依赖默认（`--mbc` 自动按卡型给 5V）。
> ⚠️ 头部 live 读取尚未接通，`info` 暂只认 GBA；MBC 仅写/清/导可测。

- [ ] `cfb erase --mbc` 整片擦除 GB 卡，不报错
- [ ] `cfb dump --mbc --out gb_backup.gbc --len <字节>` 导出（分页正确，数据合理）
- [ ] `cfb burn --mbc --rom <小测试.gb>` 烧录；结束报 `成功 ✅`
- [ ] 烧录后 `cfb dump --mbc` 回读与源文件一致（手动比对）
- [ ] 跨 bank（ROM > 16KB）写入/读出正确（验证 bank 切换 0x2000/0x3000）

## 4. JS 客户端集成（NDJSON）

- [ ] `cfb burn --rom <f> --json` 输出逐行事件：`log` / `progress` / `result`
- [ ] stdout 全是合法 JSON（诊断在 stderr），客户端能逐行 `JSON.parse`

## 已知待办（非本轮）

- [ ] GB 卡**头部 live 读取**：移植 GB 总线读头，让 `info --mbc` 显示 GB 标题/MBC 类型/RTC
- [ ] GBA RTC 真正的 GPIO/S3511 探测（现为 GameCode 启发式）
- [ ] RAM/存档 读写（参考源 `0xf7/0xf8` 与 `mission_*` 的 RAM 部分）
