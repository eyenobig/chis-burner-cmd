# cfb client protocol (schema v1)

`cfb --json` uses NDJSON on stdout so desktop clients can parse events incrementally. Human-readable output remains the default when `--json` is absent.

## Transport rules

1. Emit one UTF-8 JSON object per stdout line, without a BOM.
2. Every object has a string `type` discriminator and snake_case fields.
3. Keep stdout event-only. Unstructured diagnostics and warnings go to stderr; structured `diagnostic` events use stdout in JSON mode.
4. Field types and meanings are stable. Compatible changes may add event types or optional fields.
5. Process-level failures use a non-zero exit code. Device and operation failures should also emit an `error` or unsuccessful `result` event.

## Events

| `type` | Purpose | Main fields |
| --- | --- | --- |
| `port` | One detected burner | `port`, `vid`, `pid`, `burner`, `open`, `name`, optional `serial` |
| `summary` | Command summary | `command`, `burners` |
| `udev` | `cfb udev` serial-permission install progress (Linux) | `phase` (`prompt` / `installed` / `already` / `canceled` / `manual` / `skipped` / `failed`), `message` |
| `selected` | Persisted port selection | `port` |
| `info` | Cartridge and flash information | `port`, `present`, `kind`, flash geometry, ROM metadata |
| `diagnostic` | Incremental `info` diagnostic record | `command`, `trace_id`, `pid`, `port`, `platform`, `attempt`, `stage`, `status`, `elapsed_ms`, `stage_elapsed_ms`, `message` |
| `error` | Command or device error | `command`, `message` |
| `progress` | Long-operation byte progress | `done`, `total` |
| `log` | Long-operation status | `message` |
| `result` | Final burn/erase/dump result | `command`, `ok`, `bytes`, `mismatch_bytes`, `seconds` |
| `voltage` | Current voltage preference | `voltage` |
| `version` | `cfb version` output | `version` |
| `rtc_data` | Cartridge RTC data | `ok`, `kind`, date/time fields |
| `save_info` | Located save (save-dump/write/verify/erase) | `save_type`, `offset`, `size` |
| `save_probe` | Save chip probe (save-probe) | `health`, `jedec`, `chip`, `save_type`, `size_bytes`, `bank_size`, `banks`, `mirrored`, `writable`, `write_probed`, `restored`, `notes` |

The executable schema is the `Event` enum in [`src/event.rs`](../src/event.rs).

## Cartridge identification diagnostics

`info --json` emits `diagnostic` events immediately, including before blocking serial calls.
Older clients may ignore this additive event type. Buffer business `info` events if needed,
but forward diagnostics as they arrive, including when the process eventually times out.

`trace_id` identifies one CLI invocation; `attempt` is the CLI retry number (0 during startup).
`elapsed_ms` measures the invocation; `stage_elapsed_ms` measures the named stage.
Stages cover process startup, port resolution, lease acquisition, OS port opening, reset,
power, warm-up, GBA JEDEC/CFI, GB/GBC header/CFI, GBA header, RTC and idle-power cleanup.
Statuses are `start`, `done`, `ok`, `failed`, `skipped`. `done` means the operation returned,
not that the hardware acknowledged it; inspect `failed` records in that stage.
An unavailable RTC is reported as `rtc_present=false`, not as proof of a serial failure.
Low-level failures record operation code, address, expected/received byte counts and timeout,
without ROM/save contents. A stage `start` without completion shows where the process stopped.
These records diagnose failures; they do not change identification, retries or voltage behavior.

## `save-probe`

`cfb save-probe [--mbc] [--no-write]` identifies the save chip instead of guessing. `health` is
`ok` / `unstable` / `no_response`; `unstable` means repeated reads of one address disagree, which is
almost always a cartridge that is not fully seated, and the probe then refuses to run write probes.

Probing writes to the cartridge: the JEDEC identify sequence and the bank markers land in the save
data area on SRAM/FRAM. Every touched byte is backed up, restored, and verified by read-back;
`restored: false` means the on-cart save may be modified and the command exits non-zero (4).
`--no-write` issues no write commands at all, so it only reports `health` and leaves
`save_type` / `size_bytes` unset.

Exit codes: `0` healthy, `3` unstable or no response, `4` probe bytes could not be restored.

## Typical stream

```jsonl
{"type":"progress","done":1048576,"total":33554432}
{"type":"log","message":"Writing sector 8"}
{"type":"result","command":"burn","ok":true,"bytes":33554432,"mismatch_bytes":0,"seconds":92.0}
```

## Compatibility checklist

- Register new events in this document and `src/event.rs` before client use.
- Do not rename or remove existing fields in schema v1.
- Keep unstructured diagnostics off stdout; use the `diagnostic` schema for structured records.
- Test that every stdout line independently passes `JSON.parse`.
- Update the desktop client (e.g. [beggar_chis](https://github.com/eyenobig/beggar_chis) `src/services/cfb/client.js`) when exposing a new operation. Event schema summary: [`.claude/skills/cfb-output/SKILL.md`](../.claude/skills/cfb-output/SKILL.md).
