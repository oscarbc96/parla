---
name: gpui-pitfalls
description: Use when writing or reviewing Parla GPUI (GPUI Fast / gpui-fast) code - click handlers and actions, popovers and menus, accessibility, focus, render cost, timers, secrets, and GPUI tests. Hard-won rules from this codebase, each with the why and the fix.
---

# GPUI pitfalls

Each rule: what to do, why, where it is done right. Search the code (rg) for
the cited names.

## Actions from clicks

- **Do not call `cx.dispatch_action` (App) in a click handler.** Why: the
  clicked window is already being updated, so GPUI logs "window not found"
  and drops it. Fix: call the function directly, e.g.
  `crate::app_state::open_settings(cx)` (`crates/parla/src/app_state.rs`,
  test `crates/parla/tests/settings_triggers.rs`), or use
  `window.dispatch_action(Box::new(..), cx)` (`root_view.rs` OpenAuditLog).
  Test the real click, not the action.

## Popovers, menus, selects

- **Float them: `gpui::deferred(gpui::anchored()..)`.** Why: an inline child
  takes layout space and pushes its siblings. Fix:
  `crates/parla/src/root_view.rs` (notify menu),
  `crates/components/src/composer.rs`, `reminders_view.rs`; or
  `deferred(div().absolute()..)` (`call_bar.rs` `device_menu`).
- **Close on click-outside and Escape.** `.on_mouse_down_out(..)` plus
  `.on_key_down` on `"escape"` with `cx.stop_propagation()` (`call_bar.rs`).

## Accessibility and focus

- **Label custom controls.** `.role(gpui::Role::..)`, `.aria_label(..)`,
  `.aria_value(..)` exist on `div` (gpui `elements/div.rs`). Why: a styled div
  is invisible to screen readers. Examples: `components/src/onboarding.rs`
  (RadioGroup/RadioButton), `composer.rs` (ProgressIndicator).
- **Every interactive element is a tab stop with the ring.**
  `.tab_stop(true).focusable().focus_visible_style(focus_style(colors))`; use
  `components::controls::button`/`button_frame`. A disabled control drops the
  tab stop and the pointer (`controls.rs` `.when(enabled, ..)`).

## Render cost

- **No per-frame allocations in `render`.** Why: render runs every frame and
  the `alloc_per_*` perf budgets count allocations. Fix: static keys
  (`icon.rs` `atlas_key` is `&'static str`), build strings and `SharedString`s
  when the data changes, not per render; draw hover-only chrome on demand
  (`docs/design/README.md`, the sidebar "+").
- **Repaint only what changed.** Observe the narrowest entity and `notify`
  only the view that shows it. Test it: `components/tests/sidebar.rs`
  `unrelated_session_notification_does_not_repaint_the_sidebar` counts renders.

## Timers

- **No polling, no idle timers.** One timer for the earliest deadline;
  re-arm only when the data changes or it fires; dropping the `Task` cancels
  it. Examples: `parla/src/reminders.rs` `arm_reminder_wake`,
  `parla/src/workspaces.rs` (lean timer), `chime_signal/src/machine.rs`
  `next_deadline`.

## Secrets

- **Wipe and redact.** Hold secrets in a type that zeroizes on drop and has a
  redacted `Debug`, no `Clone`/`Display`/`Serialize`: `domain/src/secret.rs`
  `SecretText`. Use `zeroize::Zeroizing` for temporaries
  (`parla/src/dev_credentials.rs`).

## Tests

- GPUI test context: `#[gpui::test] fn t(cx: &mut TestAppContext)`, a window
  with `cx.add_window_view(..)`, then `run_until_parked()` and a draw.
- Click what is drawn: `vcx.debug_bounds("<debug_selector>")` then
  `vcx.simulate_click(b.center(), Modifiers::none())`
  (`parla/tests/settings_triggers.rs`). Keys: `vcx.simulate_keystrokes("down")`.
- Layout must not jump: read bounds before and after and assert equal
  (`components/tests/search_panel.rs` "the width does not jump").

## GPUI Fast API notes

GPUI Fast (`gpui-fast` 0.1.19) is gpui-pre 0.3.14 plus retained views and
scroll layers.

- **A retained view must `cx.notify()` when state outside entities and
  globals, or the clock, changes what `render` shows.** Why: a retained view
  replays its last paint until notified. Fix: notify at the change; examples:
  `files_tab` `load_more`, the timeline status expiry.
- **Hover is frozen 100 ms after a wheel event.** Why: scroll layers replay
  while scrolling. Tests that hover right after a scroll can disable the
  freeze (`set_hover_freeze(false)`).
- **Scroll layers allocate GPU tiles for `div` scroll areas** (up to 64 MB per
  window, a `mem.idle1` risk); lists replay row paint ops without tiles.
- **Diagnose with env switches:** `GPUI_VIEW_RETENTION=0` turns retained views
  off, `GPUI_SCROLL_LAYERS=0` turns scroll layers off. Stale paint that
  disappears with retention off is a missing `cx.notify()`.

- **Parse dynamic key strings with `Keystroke::parse_with_os(&str, os)`.**
  Why: gpui-pre's `Keystroke::parse` takes `&'static str`. Fix: pass the
  platform's OS, e.g. `cx.platform_info().os` in tests or `os.gpui()` in
  `settings/src/keymap.rs`.
- **Convert colours with the `themes::color` helpers** (`rgb_to_hsla`,
  `hsla_to_rgba`, `themes/src/color.rs`). Why: gpui-pre has no `ColorExt`;
  `Rgba` and `Hsla` use plain `r g b a` and `h s l a` fields.
