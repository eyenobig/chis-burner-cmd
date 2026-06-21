# chis-burner-cmd

碳酸丐烧录器的命令行版 `cfb`：串口协议 + 烧录引擎 + 命令行，跨平台（Windows / macOS / Linux）。

本仓库正在从 C# 迁移到 **Rust**：

```
chis-burner-cmd/
├─ src/            ← Rust 实现（主线，仓库根即 Cargo 工程）
│  ├─ main.rs        CLI 入口与子命令派发
│  ├─ device/        设备识别（data 数据集 + ops 实现函数）
│  ├─ rom/           ROM / 卡带操作
│  │  ├─ common/       通用（CartridgeKind / is_blank / GameName）— data + ops
│  │  ├─ gba/          GBA：flash 读取、GBA 判别、头解析 — data + ops
│  │  ├─ mbc/          MBC (GB/GBC)：头解析 + maptype（就绪，live 读取待移植）— data + ops
│  │  └─ mod.rs        cfb info 命令编排
│  ├─ cartridge_link.rs   串口协议层（USB CDC 收发/上电/复位/重连）
│  ├─ event.rs       NDJSON 事件定义
│  ├─ config.rs      select 选择的持久化（~/.cfb.json）
│  ├─ i18n.rs        语言包加载
│  └─ i18n/          zh-CN.json / en.json
├─ docs/           ← 功能说明文档（read-id.md 等）
├─ Cargo.toml
├─ driver/         ← 可选 INF：仅 Windows，给设备一个友好名（平台中立目录）
└─ README.md
```

> 每个平台子模块（device / rom/gba / rom/mbc / rom/common）都按 **`data`(数据集) + `ops`(实现函数)** 拆分。
> ROM 平台的 `ops/` 再按 **读 / 写 / 删 / 导**（`read`/`write`/`delete`/`export`）分文件（目前仅 `read` 已实现）。
> 电压（3.3V/5V 识别与控制，`Voltage`/`power`/`voltage_for`）归 `device`；底层 `0xa0` 发包在 `cartridge_link`。

## Rust 版（主线）

```bash
cargo build --release        # 产物: target/release/cfb(.exe)
cargo run -- detect          # 人类可读（只列烧录器）
cargo run -- detect --json   # NDJSON 事件流（给 JS 客户端解析）
cargo run -- --lang en detect
```

子命令（省略 `--port` 时优先用 `select` 记住的烧录器，否则按 VID/PID `0483/0721` 自动识别）：

| 命令 | 说明 | 状态 |
|------|------|------|
| `detect [--json]` | 列出**已连接的烧录器**（非烧录器串口不显示） | ✅ 已实现 |
| `select [--port P] [--clear]` | 选择并记住一个烧录器（多台时），存 `~/.cfb.json` | ✅ 已实现 |
| `voltage [3v3\|5v\|off\|auto]` | 记住/查看供电电压偏好（持久化） | ✅ 已实现 |
| `info` | 读 flash 芯片 + 卡带/游戏信息（ID/容量、GBA 判别、RomTitle/GameCode/Revision/RTC/RomChecksum，[文档](docs/read-id.md)） | ✅ 已实现 |
| `burn --rom <f> [--mbc]` | 写入 ROM（GBA / GB·GBC） | 🟡 已实现，待硬件测试 |
| `erase [--mbc]` | 清空 ROM（整片擦除） | 🟡 已实现，待硬件测试 |
| `dump --out <f> [--mbc]` | 导出 ROM 到文件 | 🟡 已实现，待硬件测试 |
| `help` | 显示帮助 | ✅ |

**端口选择优先级**：显式 `--port` > `cfb select` 记住的(仍在线) > 自动第一个烧录器。
多台烧录器时 `cfb select` 列出供选择并记住；`--json` 模式不交互，需 `--port` 指定。

**语言**：`--lang zh-CN|en`，缺省跟随系统、回退中文。文案走语言包 [src/i18n/](src/i18n/)
（`zh-CN.json` / `en.json`），加语言只需加一个 json。

### 给 JS 客户端的输出（Electron / Tauri）

全局 `--json` 开关让命令输出 **NDJSON 事件流**——每行一个 `{"type":...}`，客户端逐行
`JSON.parse` 即可流式展示（含将来 burn 的实时进度）：

```
{"type":"port","port":"COM7","vid":"0483","pid":"0721","burner":true,"open":true,"name":"USB 串行设备 (COM7)"}
{"type":"summary","command":"detect","burners":1}
```

格式是稳定契约，定义见 [.claude/skills/cfb-output/SKILL.md](.claude/skills/cfb-output/SKILL.md) 与
[src/event.rs](src/event.rs)；新增命令请遵循它，保证客户端稳定解析。

设备识别用 [`serialport`](https://crates.io/crates/serialport) crate，它在三大平台统一提供 USB
VID/PID，无需像 C# 版那样按平台分别走 WMI/ioreg//sys。注意 `detect` 只列出**当前实际接入**的端口
（不显示已拔出的历史/幽灵设备）。

## 复刻参考源

协议/烧录逻辑从外部 C# 工程 **`Z:\Project\beggar_socket\client`**（ChisFlashBurner 上位机，
WinForms/.NET Fx 4.8，只读参考）复刻而来。源文件 → Rust 落点映射、移植做法见技能
[.claude/skills/cfb-port-source/SKILL.md](.claude/skills/cfb-port-source/SKILL.md)。

## 设备驱动说明

烧录器是 STM32 的 **USB CDC（虚拟串口）**，Windows/macOS/Linux 都用**系统自带驱动**，无需安装。
`driver/` 下的 INF 仅用于在 Windows 设备管理器里改个友好名（可选，仅 Windows）；跨平台统一显示名
应改固件的 USB iProduct 字符串，详见 [driver/README.md](driver/README.md)。
