---
name: cfb-output
description: cfb 命令行的输出格式契约（NDJSON 事件流 + 人类可读双模式）。实现或修改 cfb 任何子命令（detect/info/burn/...）的输出时必读必守，确保 Electron/Tauri 等 JS 客户端能稳定逐行解析。改动事件 schema 前也读这里。
---

# cfb 输出格式契约 (schema v1)

cfb 的每个子命令有**两套输出**，必须并行维护：

- **人类可读（默认）**：精简、对终端友好。无固定机器格式，可随意调整排版。
- **`--json`（NDJSON 事件流）**：给 JS 客户端解析。**这套有稳定契约，改动需遵守下面的规则。**

实现在 [src/event.rs](../../../src/event.rs)（事件定义 + `emit`）；各命令在 `src/*.rs` 里按 `json: bool` 分两支输出。

## NDJSON 规则（硬约束）

1. **一行一个 JSON 对象**，输出到 **stdout**，行尾 `\n`。客户端逐行 `JSON.parse`。
2. 每个对象**必须有 `type` 字段**（字符串）做判别；客户端按 `type` 分发。
3. 字段名 **snake_case**；编码 **UTF-8**；不输出 BOM。
4. **stdout 只放事件**。诊断/调试/警告走 **stderr**，绝不混进 stdout，否则会污染 NDJSON 流。
5. 命令正常跑完即 **exit 0**；“没找到烧录器”“芯片读不出”等是**数据/error 事件**，不是非零退出。仅进程级失败（参数错误、IO 致命）才非零。
6. 字段值类型固定：`vid`/`pid` 是 4 位大写十六进制字符串（如 `"0483"`）或 `null`；地址/容量是数字（字节）；布尔就是布尔。

## 事件目录（v1）

> 新增事件或字段时，**先在此表登记**再写代码，并同步更新 `src/event.rs`。

| type | 何时发 | 字段 |
|------|--------|------|
| `port` | detect：每个**烧录器**一条（detect 已过滤，非烧录器串口不输出） | `port` `vid?` `pid?` `burner`(恒 true) `open`(bool) `name` |
| `summary` | 命令结束的汇总（detect 只统计烧录器） | `command` `burners` |
| `selected` | select：记住的端口；`port=null` 表示已清除 | `port`(string\|null) |
| `error` | 未实现/未知命令/设备异常/select 需 --port | `command` `message` |
| `info` | info：flash 芯片 + 卡带/游戏信息 | `port` `present`(bool) `kind`("gba"/"gb_mbc"/"unknown") `id` `capacity_bytes`(u64) `buffer_write_bytes` `sector_size` `sector_count` `game_name?` `rom_title?` `game_code?` `revision?` `rom_checksum?`{`stored`,`computed`,`ok`} `rtc?`(bool)。游戏字段仅识别到 GBA 游戏时非 null |
| `progress` | burn/dump：进度 | `done` `total`（字节） |
| `log` | burn：阶段性日志 | `message` |
| `result` | burn/erase/dump：最终结果 | `command` `ok`(bool) `bytes`(u64) `mismatch_bytes` `seconds` |
| `voltage` | voltage：当前/设置的电压偏好 | `voltage`("3.3V"/"5V"/"off"/"auto") |

### 典型流

```
# cfb detect --json   （只列烧录器）
{"type":"port","port":"COM7","vid":"0483","pid":"0721","burner":true,"open":true,"name":"USB 串行设备 (COM7)"}
{"type":"summary","command":"detect","burners":1}

# cfb select --json --port COM7   （--json 不交互，必须给 --port）
{"type":"selected","port":"COM7"}

# cfb burn --rom x.gba --json  (预留形态)
{"type":"info","port":"COM7","id":"...","capacity_bytes":33554432,...}
{"type":"progress","done":1048576,"total":33554432}
{"type":"result","ok":true,"bytes_written":33554432,"mismatch_bytes":0,"seconds":92}
```

## 端口选择与持久化

- 省略 `--port` 时端口解析优先级：**显式 `--port` > `cfb select` 记住的(仍在线) > 自动第一个烧录器**。
- `cfb select` 把选定端口写入 `~/.cfb.json`（`{"port":"COM7"}`）；`--clear` 清除；只有一台时自动记住。
- `--json` 模式下 `select` **不弹交互**，须 `--port` 指定，否则发 `error` 事件。
- 解析过程的提示（“使用已记住的端口…”/“自动选择…”）一律走 **stderr**，不进 NDJSON 流。

## 国际化（i18n）

- 所有面向用户的文案走语言包 [src/i18n/](../../../src/i18n/)（`zh-CN.json` / `en.json`，`include_str!` 嵌入）。
- 取值：`i18n::t("key")`；带占位用 `i18n::tf("key", &[("name","值")])`，模板里写 `{name}`。
- 语言：`--lang zh-CN|en`；缺省跟随系统（读 `LANG`/`LC_ALL`），回退 `zh-CN`。
- **新增任何用户可见字符串都要进语言包**（两套都加），不要在代码里硬编码中文/英文。
- 注意：设备名 `name` 字段可能来自操作系统的 USB product 字符串（如 Windows 提供的本地化名），不归 i18n 管。

## 兼容性规则（不要破坏客户端）

- **只做加法**：可新增 `type`、可给现有事件**追加可选字段**。
- **不要**重命名/删除已有字段，不要改字段类型或语义。
- 必须破坏性变更时：升 schema 版本，并在本文件记录迁移说明（客户端据 `summary`/首事件判别版本）。

## 加新命令的清单

1. 在上表登记该命令要发的事件与字段。
2. 在 `src/event.rs` 的 `Event` 枚举加对应变体（`#[serde(rename_all="snake_case")]` 已统一命名）。
3. 命令实现按 `json` 分两支：`--json` 走 `emit(&Event::…)`；否则人类可读。
4. **所有用户可见文案进语言包**（`src/i18n/zh-CN.json` + `en.json` 都加），代码里用 `i18n::t`/`tf`。
5. 端口相关命令用 `device::resolve_port(--port)` 解析，遵循 select 优先级；诊断走 stderr。
6. 诊断只往 stderr；正常完成 exit 0。
7. 用 `cfb <cmd> --json | <逐行 JSON.parse>` 自测每行合法（见 README/本仓库测试）。
