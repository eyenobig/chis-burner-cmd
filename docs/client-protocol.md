# cfb client protocol (schema v1)

`cfb --json` uses NDJSON on stdout so desktop clients can parse events incrementally. Human-readable output remains the default when `--json` is absent.

## Transport rules

1. Emit one UTF-8 JSON object per stdout line, without a BOM.
2. Every object has a string `type` discriminator and snake_case fields.
3. Keep stdout event-only. Diagnostics and warnings go to stderr.
4. Field types and meanings are stable. Compatible changes may add event types or optional fields.
5. Process-level failures use a non-zero exit code. Device and operation failures should also emit an `error` or unsuccessful `result` event.

## Events

| `type` | Purpose | Main fields |
| --- | --- | --- |
| `port` | One detected burner | `port`, `vid`, `pid`, `burner`, `open`, `name` |
| `summary` | Command summary | `command`, `burners` |
| `selected` | Persisted port selection | `port` |
| `info` | Cartridge and flash information | `port`, `present`, `kind`, flash geometry, ROM metadata |
| `error` | Command or device error | `command`, `message` |
| `progress` | Long-operation byte progress | `done`, `total` |
| `log` | Long-operation status | `message` |
| `result` | Final burn/erase/dump result | `command`, `ok`, `bytes`, `mismatch_bytes`, `seconds` |
| `voltage` | Current voltage preference | `voltage` |
| `version` | `cfb version` output | `version` |
| `rtc_data` | Cartridge RTC data | `ok`, `kind`, date/time fields |
| `save_info` | Located save (save-dump/write/verify) | `save_type`, `offset`, `size` |

The executable schema is the `Event` enum in [`src/event.rs`](../src/event.rs).

## Typical stream

```jsonl
{"type":"progress","done":1048576,"total":33554432}
{"type":"log","message":"Writing sector 8"}
{"type":"result","command":"burn","ok":true,"bytes":33554432,"mismatch_bytes":0,"seconds":92.0}
```

## Compatibility checklist

- Register new events in this document and `src/event.rs` before client use.
- Do not rename or remove existing fields in schema v1.
- Keep diagnostics off stdout.
- Test that every stdout line independently passes `JSON.parse`.
- Update `beggar_chis/src/services/cfb/client.js` when exposing a new operation to the desktop client.