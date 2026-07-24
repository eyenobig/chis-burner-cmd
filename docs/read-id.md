# 读取卡带信息（`cfb info`）

读取烧录器上的 flash 芯片信息与卡带/游戏信息：

- **flash 芯片**：Autoselect ID + CFI 容量/写缓冲/扇区（从 `GbaFlasher.cs ReadInfo()` 复刻）。
- **有无卡带**：flash 是否有有效响应。
- **卡带类型**：GBA / GB-GBC(MBC) / 未识别。
- **GBA 游戏头**：GameName、RomTitle、GameCode、Revision、RTC、RomChecksum。

参考源：`Z:\Project\beggar_socket\client`（`ChisFlashBurner.Core` + `mission_gba.cs` / `utility.cs`）。

## 用法

```bash
cfb info                 # 自动选端口，人类可读
cfb info --port COM7     # 指定端口
cfb info --json          # NDJSON 事件（给 Electron/Tauri 客户端）
cfb --lang en info       # 英文
```

端口解析优先级：`--port` > `cfb select` 记住的(仍在线) > 仅一台时自动；多台未选择则失败。

## 输出

**人类可读（插了 GBA 游戏卡时）：**

```
端口:     COM7
卡带:     已检测到 (GBA)
芯片 ID:  C2 22 28 22 01 22 00 22
容量:     33554432 字节 (32 MB)
写缓冲:   512 字节
扇区:     131072 字节 × 256
---- 游戏 ----
游戏名:   POKEMON EMER
标题:     POKEMON EMER
GameCode: BPEE
版本:     0
RTC:      有
头校验:   通过 (0x1F)
```

**`--json`（一条 `info` 事件）：**

```json
{"type":"info","port":"COM7","present":true,"kind":"gba","id":"C2 22 28 22 01 22 00 22","capacity_bytes":33554432,"buffer_write_bytes":512,"sector_size":131072,"sector_count":256,"game_name":"POKEMON EMER","rom_title":"POKEMON EMER","game_code":"BPEE","revision":0,"rom_checksum":{"stored":31,"computed":31,"ok":true},"rtc":true}
```

游戏字段（`game_name`/`rom_title`/`game_code`/`revision`/`rom_checksum`/`rtc`）**仅识别到 GBA 游戏时非 null**；
空片或非 GBA 时为 null。事件契约见 [client-protocol.md](client-protocol.md)。

## 判别与解析逻辑

| 项 | 做法 |
|----|------|
| **有无卡带** | flash Autoselect ID 有有效响应且非全 `0xFF`（`presence::flash_present`） |
| **GBA 判别** | 读 GBA 总线头(0xC0B)，`header[0xB2]==0x96` 且头补码校验通过 → GBA |
| **RomTitle** | 0xA0..0xAB（12B ASCII） |
| **GameCode** | 0xAC..0xAF（4B ASCII） |
| **Revision** | 0xBC（1B） |
| **RomChecksum** | stored=header[0xBD]；computed = `-(0x19 + Σ header[0xA0..=0xBC]) & 0xFF`；报告两值与是否一致 |
| **RTC** | **启发式**：按已知带 RTC 的 GameCode 前缀（AXV/AXP/BPE/U3I/U32/U33/BKA/BR4）判断。真正的 GPIO/S3511 探测待移植 |
| **GameName** | 参考源无名称数据库，回退到 RomTitle（`name::game_name`，预留查表位） |

> **GBA vs MBC**：参考源是按文件扩展名 + 用户手选；这里改为从卡内 ROM 头自动判别 GBA。
> **MBC(GB/GBC) 的 live 读取尚未实现**——读物理 GB 卡需要 GB 总线协议（`gbcCart_read` + 分页，
> 见 `cart_adapter.cs`），还没移植进 `cartridge_link`。`src/rom/mbc` 的**解析逻辑已就绪**，
> 等 GB 读取接通即可用。

## 退出码与错误

| 情况 | 退出码 | 说明 |
|------|--------|------|
| 成功 | 0 | 输出 `info` 事件 / 信息 |
| 找不到烧录器 | 2 | 未检测到 VID 0483/PID 0721 |
| 打开端口失败 | 3 | 端口被占用或不可用（`info.err_open`） |
| 未检测到卡带 | 3 | flash 无响应/全 `0xFF`（多为没插卡/没上电）；`--json` 发 `error` 事件 |

诊断（“自动选择端口…”等）走 **stderr**，不污染 `--json` 的 stdout 事件流。

## 源码

协议层 [../src/cartridge_link.rs](../src/cartridge_link.rs)；卡带逻辑在 `src/rom/`，按平台分 + `data`/`ops` 拆分：

- [../src/rom/common/](../src/rom/common/)（通用）：`data` = `CartridgeKind`；`ops` = `is_blank` / `game_name`
- [../src/rom/gba/](../src/rom/gba/)（GBA）：
  - `data` = `FlashInfo` / `GbaHeader`
  - `ops` 按 **读/写/删/导** 分文件：`read.rs`（`read_info` ID+CFI / `flash_present` 有无卡带 / `is_gba_header` 判别 / `header_checksum` / `has_rtc` / `parse_header`，已实现）、`write.rs`/`delete.rs`/`export.rs`（待移植）
- 供电电压在 [../src/device/](../src/device/)：`Voltage`(3.3V/5V/off) + `power`/`power_off`/`voltage_for`（按卡型识别电压）；底层 `0xa0` 发包在 `cartridge_link::power`
- [../src/rom/mbc/](../src/rom/mbc/)（GB/GBC，就绪待接 GB 读取）：
  - `data` = `MbcHeader` + **maptype**（`mbc_name`：cartridge type → MBC 名称，GB 独有）
  - `ops` = `parse_header` / `header_checksum` / `has_rtc`
- [../src/rom/mod.rs](../src/rom/mod.rs)：`cfb info` 命令编排
- 解析逻辑单测：`src/rom/mod.rs` 的 `#[cfg(test)]`（`cargo test`，无需硬件）
