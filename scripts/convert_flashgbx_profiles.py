#!/usr/bin/env python3
"""把 flashGBX 的 fc_*.txt 转成 chis-burner-cmd 的 profile JSON。

用法:
    python scripts/convert_flashgbx_profiles.py <flashgbx_config_dir> <output_dir>

差异处理（flashGBX → cfb）:
- names (复数数组) → name (取 names[0]，其余进 aliases)
- flash_size/sector_size 的 hex 字面量（严格 JSON 非法）→ 十进制
- 保留所有 commands.* 序列（cfb 的 profile.rs 解析器兼容 flashGBX 的 null/数字/字符串混用）
- flashGBX 额外字段（rtc/rumble/enable_pullups/write_pin/mbc/...）保留进 extra
- 文件名 fc_AGB_xxx.txt → agb/xxx.json，fc_DMG_xxx.txt → dmg/xxx.json

数据源: https://github.com/lesserkuma/FlashGBX/tree/master/FlashGBX/config
许可归属: flashGBX (Lesserkuma)，见本仓库 README。
"""
import json
import os
import re
import sys
import glob

if len(sys.argv) != 3:
    print("用法: python convert_flashgbx_profiles.py <flashgbx_config_dir> <output_dir>")
    sys.exit(2)

SRC_DIR, OUT_DIR = sys.argv[1], sys.argv[2]
os.makedirs(os.path.join(OUT_DIR, "agb"), exist_ok=True)
os.makedirs(os.path.join(OUT_DIR, "dmg"), exist_ok=True)

# 预处理：把 JSON 里裸 hex 字面量 (0xNNN) 转十进制，让标准 JSON 解析器能吃。
HEX_RE = re.compile(r'(?<!["\w])0[xX][0-9A-Fa-f]+')


def fix_hex_literals(text):
    return HEX_RE.sub(lambda m: str(int(m.group(0), 16)), text)


# flashGBX 额外字段（cfb 不直接用，但保留信息）。
EXTRA_KEYS = (
    "rtc", "rumble", "enable_pullups", "write_pin", "mbc", "start_addr",
    "first_bank", "double_die", "flash_commands_on_bank_1",
    "pulse_reset_after_write", "3d_memory", "page_write",
)

converted = 0
skipped = []
for path in sorted(glob.glob(os.path.join(SRC_DIR, "fc_*.txt"))):
    fname = os.path.basename(path)
    raw = open(path, encoding="utf-8").read()
    try:
        data = json.loads(fix_hex_literals(raw))
    except Exception as e:
        skipped.append((fname, f"parse: {e}"))
        continue

    if fname.startswith("fc_AGB_"):
        kind, type_tag = "agb", "AGB"
        stem = fname[len("fc_AGB_"):]
    elif fname.startswith("fc_DMG_"):
        kind, type_tag = "dmg", "DMG"
        stem = fname[len("fc_DMG_"):]
    else:
        skipped.append((fname, "unknown kind"))
        continue

    names = data.get("names", [])
    name = names[0] if names else stem.replace(".txt", "")

    out = {
        "name": name,
        "type": type_tag,
        "flash_ids": data.get("flash_ids", []),
        "voltage": data.get("voltage", 3.3 if kind == "agb" else 5.0),
        "flash_size": data.get("flash_size", 0),
        "sector_size": data.get("sector_size", 0),
        "sector_size_from_cfi": data.get("sector_size_from_cfi", False),
        "chip_erase_timeout": data.get("chip_erase_timeout", 200),
        "command_set": data.get("command_set", ""),
        "commands": data.get("commands", {}),
    }
    if len(names) > 1:
        out["aliases"] = names[1:]
    extra = {k: data[k] for k in EXTRA_KEYS if k in data}
    if extra:
        out["extra"] = extra

    out_path = os.path.join(OUT_DIR, kind, stem.replace(".txt", "") + ".json")
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=2)
        f.write("\n")
    converted += 1

print(f"转换完成: {converted} 个 profile")
if skipped:
    print(f"跳过 {len(skipped)} 个:")
    for fname, why in skipped:
        print(f"  {fname}: {why}")
