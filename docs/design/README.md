# Parla design tokens

The tokens and the rules in this folder are the public source of truth for
Parla's look. The design boards the app was drawn from are the maintainer's
reference and are not published; the tokens, the built-in themes, the
`design-system` skill and the visual baselines under
`crates/ui_testing/baselines` show the expected result.

## Files

- `parla.tokens.json`: the design tokens in the W3C Design Tokens format. This
  is the **source of truth**. Primitives come from the Parla logo (navy
  `#211E4C`, amber `#FFA611`, orange `#FE6129`, coral `#FD3459`); semantic
  tokens per theme (`theme.light`, `theme.dark`) reference the primitives.
- `tokens.css`: CSS variables from the tokens, for the site and for mockups.
  No generator exists: keep it in step by hand, in the same change as the tokens.

## Built-in themes

`assets/themes/parla.json` ships the built-in themes **Parla Light** and **Parla
Dark** (the defaults). They mirror the tokens file. `assets/themes/parla-high-contrast.json` ships
**Parla High Contrast Light** and **Parla High Contrast Dark** (tokens
`theme.high-contrast-light` and `theme.high-contrast-dark`): every text pair
at 7:1 or more (WCAG AAA), focus rings and control borders at 4.5:1, other
borders and indicators at 3:1, checked by `crates/themes/tests/high_contrast.rs`.
They replace Parla Light and Parla Dark while macOS "Increase contrast" is on
and the user has not set `theme.light` / `theme.dark`, and can be picked by
name anywhere. HC Light has a light sidebar so badges, presence and rings hold
3:1 and 4.5:1 there too. The test `brand_themes_match_design_tokens` in
`crates/themes/tests/themes.rs` compares the sidebar background, text,
background and element background of all four themes with the tokens, so the two
files cannot drift. Change a colour in the tokens first, then in the theme.

## Fork a theme

Copy one theme object from `assets/themes/parla.json` (for example `Parla Dark`)
into a theme-family file in your user `themes/` folder, give it a new `name`,
and edit the colours. Parla reloads it live; select it by name under `theme` in
`settings.json`. See the themes page of the user docs for the file shape.

## Rules for contributors and agents

UI code takes every colour from the active theme and every size from
`themes::metrics` (generated from the tokens; `crates/themes/tests/metrics.rs`
keeps them equal). Raw `rgb`/`hsla`/`#hex` and off-scale `px(..)` in
`crates/components` and `crates/parla` fail `mise run lint:design`. A missing
token is added here first, then to the themes. The full rules, the token to
Rust map and the board to code map are in the `design-system` skill
(`.agents/skills/design-system/SKILL.md`); the `design-reviewer` subagent
checks UI PRs against them.

## Deviations from the boards

Where the app differs from a board on purpose, the reason is recorded here.

| Board | Difference | Reason |
| --- | --- | --- |
| Welcome, ProviderPicker, Stepper | The add-workspace window (rail "+") shows these boards in their own window at the main window's size and minimum size, not the sign-in window's. Welcome has a Cancel button under "Get started", and the stepper reads "of 4" (no first sync or tour step). | The boards are laid out for the main window and do not reflow below it. Added workspaces join the main window, so there is nothing to sync-screen or tour; the window needs a visible way out. |
| Sidebar | The Channels and Direct messages "+" shows only while its section header is hovered or has keyboard focus (the header, or the "+" inside it); the board draws it always. Chevrons show on every foldable header, as on the board; Unreads cannot fold and has none. | Performance: an always-drawn "+" on every header costs about 110 allocations per sidebar redraw and broke the `alloc_per_long_alert_redraw` budget. Budgets change only deliberately, so the icon is drawn on demand. |
| SidebarOrganise | "Sidebar settings…" and "Organise sidebar" are entries of the workspace menu (the workspace name), not a "Sidebar settings" row at the foot of the sidebar; the popover adds an "Organise sidebar" button. Done sits on the sidebar ground (`sidebar.hover`, strong text), not the signal fill. The section menu has Rename, Move up, Move down and Delete section (no per-section sort, unreads only or mute). "Move to" is one row menu entry per section, not a submenu. The drop target is tinted `signal.soft` instead of a signal line between rows. | No always-drawn row at the foot or header button: every sidebar redraw would pay for it and every baseline would move. Signal (#FE6129) with white text is under 4.5:1. Per-section sort, unreads only and mute need their own settings and are left for later; the menu has no submenus. Moving rows within a section is not offered: a section follows the sidebar sort. |
| CodeViewer | No file tabs, font size buttons, "Follows Parla theme" chip, line selection menu or minimap; every find match is drawn in `text` on the soft signal ground (the syntax colours fall under 4.5:1 on it in Parla Light and Parla Dark), the current one adds a signal underline and semibold weight instead of a solid signal ground with dark text, and its line gets the hover ground. The language is a static outlined label, not a picker. Find has no case toggle (ASCII case is always ignored). | Scope: one file per window. A solid signal ground needs a text-on-signal token that does not exist; the underline keeps the match readable in every theme, forked ones included. |
| New messages divider | The label does not apply `tracking.caps` (0.06em letter spacing). | GPUI has no letter spacing; it is applied once GPUI supports it. |
| Titlebar | No toggle-sidebar or history button. The help button opens the docs, the search pill shows its own `cmd-shift-f` key cap (not the board's `cmd-k`, which opens the quick switcher), and the bell is crossed out while notifications are paused. | There is no hidden-sidebar state or history menu to open; both are features of their own. A key cap names the key that runs the control. The board only draws the bell, so the paused state had no shape. |
| Sidebar | The workspace name stays 17px (the board's 16px) and a section header keeps the row's height with no 14px top margin (the board's `.sb-head`). Unreads is a section header, not a quick-link row, and Mentions is not a sidebar row. | There is no 16px font token. The rows are one uniform height in the virtualised list (a budgeted hot path), so a taller header would need a variable-height list. Parla's Unreads is a foldable section and it has no all-mentions view. |
| Mentions, AppMessagesDrafts | A mention chip is drawn without its 4px corner radius (the ground follows the glyphs), and the APP tag has no 0.04em letter spacing. | GPUI text highlights paint a run's ground with no corner radius, and text has no letter spacing; both wait for GPUI support. |
