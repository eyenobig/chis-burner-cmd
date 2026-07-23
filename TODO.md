# TODO

- [x] 大 ROM（≥16MB）烧录：进度推进正常，无卡死（验证重连逻辑）—— 已硬件验证。
- [x] flashGBX 风格**外部 profile 配置**（按 flash_id 匹配，承载 flash 命令字节）。已实现 profile 命令序列外部化（reset/read_id/read_cfi/sector_erase/chip_erase），内置 + `~/.cfb/profiles/` 覆盖，格式兼容 flashGBX `fc_*.txt`；GBA 侧已硬件验证（命中 profile 走序列烧录成功）。**MBC 侧自动匹配待补**（需 MBC Autoselect ID 读取）。详见 [docs/profiles.md](docs/profiles.md)。
- [ ] GBA RTC 真正的 GPIO/S3511 探测（现为 GameCode 启发式）。
- [x] RAM/存档 读写（参考源 `0xf7/0xf8` 与 `mission_*` 的 RAM 部分）。已实现 `save-dump`/`save-write`/`save-verify`（GBA SRAM/FLASH/FRAM/免电、MBC SRAM/FRAM），**待硬件验证**。
