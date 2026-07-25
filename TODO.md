# TODO

未完成：

- [ ] GBA `info` 侧 RTC：用 GPIO/S3511 探测替代 GameCode 启发式（读 RTC 已实现，见 `gba/ops/rtc.rs`）。
- [ ] MBC 侧 flash profile **自动匹配**（需 MBC Autoselect ID 读取）；详见 [docs/profiles.md](docs/profiles.md)。
- [ ] 存档：`save-erase` 及非 GBA-SRAM 路径的硬件验证（MBC SRAM/FRAM、GBA FLASH/FRAM/免电）。
