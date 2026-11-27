---
name: app-screenshot
description: Use when changing any Parla UI or debugging "what does the user see" - a layout, theme, sidebar, message-list or any app bug that shows in the window - and for before/after evidence. Launches the real Parla app (fixture or real account), captures its window to a PNG under target/shots, and tells you how to read it. macOS only.
---

# App screenshot

Agents cannot see the app by reasoning about code. Capture the real window and
look at it, before and after every UI or app-behaviour fix.

## When

- Any UI change (components, themes, views, icons).
- Any bug about what the user sees or what the window does.
- Before/after evidence for a PR (fixture shots only, see Privacy).

## How

```sh
mise run app:shot -- --fixture --appearance both        # synthetic data, light and dark
mise run app:shot -- --fixture --wait-log "started" --delay 1500 --out target/shots/after.png
mise run app:shot -- --attach --out target/shots/now.png # an already running parla
```

Then Read the PNG path it prints with the Read tool and describe what you see.
It also prints the last 30 scrubbed log lines.

Options: `--out`, `--wait-log "<text>"` (stderr or `parla.log`), `--delay <ms>`,
`--keep-running`, `--binary <path>`, `--env KEY=VAL` (repeatable), `--fixture`
(same as `--env PARLA_FIXTURE=perf`), `--attach`, `--appearance light|dark|both`,
`--timeout <ms>`, `--no-build`. Run `mise run app:shot -- --help` for the list.

- First run compiles `scripts/window-id.swift` into `target/tools/window-id`.
  The default binary is always built first (incremental, fast when nothing
  changed; with the `fixture` feature when a fixture is requested), so a shot
  never shows stale code. Pass `--no-build` to skip it; `--binary` and
  `--attach` never build.
- A fixture run uses a scratch data, cache and config dir, so it never touches
  real data. `--appearance` writes `theme.mode` into that scratch settings file.
- The terminal needs the macOS Screen Recording permission.

## Fixture or real account

- Fixture (`--fixture`): synthetic `FakeProvider` workspace. Use it for
  anything reproducible and for every PR screenshot.
- Real account (no `--fixture`, or `--attach`): only to reproduce a bug that
  needs real data, and only with the owner's own session.

## Privacy

A real-account screenshot may contain private messages and names. Keep it under
`target/`, never commit it, never upload it to a PR, issue or chat, and delete
it when done. PR screenshots come from fixtures only.
