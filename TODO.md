# TODO

- [x] 大 ROM（≥16MB）烧录：进度推进正常，无卡死（验证重连逻辑）—— 已硬件验证。
- [ ] flashGBX 风格**外部 profile 配置**（按 cartridge_type/flash_id 匹配，承载 flash 命令字节）—— 阶段 2。
- [ ] GBA RTC 真正的 GPIO/S3511 探测（现为 GameCode 启发式）。
- [x] RAM/存档 读写（参考源 `0xf7/0xf8` 与 `mission_*` 的 RAM 部分）。已实现 `save-dump`/`save-write`/`save-verify`（GBA SRAM/FLASH/FRAM/免电、MBC SRAM/FRAM），**待硬件验证**。
