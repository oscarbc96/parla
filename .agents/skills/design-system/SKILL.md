---
name: design-system
description: Use before writing or reviewing any Parla UI - a component, view, window, popover, settings page, theme, icon, site page or visual baseline. Maps the design (docs/design tokens and the built-in themes) to the Rust theme and metrics APIs, lists the rules every UI change follows, and the checks that enforce them.
---

# Design system

Parla has one design system. Its source of truth is
`docs/design/parla.tokens.json` (W3C Design Tokens). Everything else follows it:

| Layer | File | Kept in sync by |
| --- | --- | --- |
| Tokens | `docs/design/parla.tokens.json` | source of truth |
| Built-in themes | `assets/themes/parla.json` (Parla Light, Parla Dark), `assets/themes/parla-high-contrast.json` (Parla High Contrast Light, Dark) | `brand_themes_match_design_tokens` and `high_contrast` tests |
| Rust colours | `themes::Theme::colors()`, `status()`, `themes::parla_tokens()` | theme loader |
| Rust sizes | `themes::metrics` (`SPACE_*`, `RADIUS_*`, `FONT_SIZE_*`, `LAYOUT_*`, `DURATION_*`) | `crates/themes/tests/metrics.rs` |
| CSS (site, mockups) | `docs/design/tokens.css` | kept by hand in step with the tokens; change it in the same PR |
| Screens | the design boards (the maintainer's reference) and the visual baselines in `crates/ui_testing/baselines` | this skill |

## Rules

1. **No raw colours in UI code.** Never write `rgb(0x..)`, `hsla(..)` numbers,
   `Hsla { .. }` or `#RRGGBB` in `crates/components` or `crates/parla`. Read the
   colour from the active theme. Users fork themes, so a literal breaks their
   theme.
2. **Sizes come from the scale.** Every `px(..)` is a token: spacing
   `SPACE_*` (2, 4, 6, 8, 12, 16, 20, 24, 32, 40, 48, 64), radius `RADIUS_*`,
   type `FONT_SIZE_*`, fixed chrome `LAYOUT_*` (titlebar 44, rail 68, sidebar
   264, detail panel 400, rows 28 compact / 34 cozy, plus the other `LAYOUT_*` chrome such as dialog,
   popover and window sizes). `0` and `1` (hairlines, sign ignored) are free.
   A bare `px(N)` passes only for a space, radius or font-size value; a
   `LAYOUT_*` value must be used by name, never as a literal.
3. **Missing token? Add it, do not hardcode.** If the design needs a colour or
   size the theme lacks (for example `signal`, `bg.sunken`, `border.focus`),
   add it to `parla.tokens.json`, the theme model (`crates/themes`), both themes
   in `assets/themes/parla.json`, and the user theme docs, in the same PR.
4. **Escape hatch, rarely.** A computed colour (avatar hue from a name) or an
   intrinsic size (an image's pixels) may carry `// design: allow <reason>` on
   the line or the line above. The reason is required and reviewed.
5. **Personalisable by default.** Theme, accent, density (compact/cozy row
   height), UI and code font and font size are user settings. Read them; never
   assume the default theme, font or density.
6. **Both appearances.** Every surface works in Parla Light and Parla Dark, and
   in a forked theme. Check both before you finish.
7. **Brand.** The logo is `assets/brand/logo.svg`; its colours come from the
   theme's `logo.*` tokens. Navy `#211E4C`, amber `#FFA611`, orange `#FE6129`,
   coral `#FD3459` live only in the tokens file. No Slack logos, colours,
   icons, fonts or sounds.
8. **Accessibility.** Text meets WCAG AA contrast on its ground in both
   themes; every interactive element is reachable by keyboard and shows the
   `border.focus` ring; icons that act have an accessible label; nothing is
   conveyed by colour alone (presence uses shape too).
9. **Text is localised.** UI strings go through `crates/**/i18n/*.ftl`, never
   inline. Layouts tolerate 40% longer strings (the `en-XA` pseudo-locale).

## Colour token map

| Design token | Rust |
| --- | --- |
| `bg.app` | `theme.colors().background` |
| `bg.surface` | `theme.colors().surface_background` |
| `bg.raised` (popovers, menus) | `theme.colors().elevated_surface_background` |
| `bg.hover` | `theme.colors().element_hover` |
| `bg.selected` | `theme.colors().element_selected` |
| `border.default` | `theme.colors().border` |
| `text.default` / `text.muted` | `theme.colors().text` / `text_muted` |
| `text.link`, `accent` | `theme.colors().text_accent` |
| `code.bg` | `theme.colors().editor_background` |
| `mention.bg` (highlighted row) | `parla_tokens(theme).message_mention_background` |
| `sidebar.*` | `parla_tokens(theme).sidebar_background`, `sidebar_text`, `sidebar_active` |
| `sidebar.rail`, `sidebar.border`, `sidebar.hover`, `sidebar.muted`, `sidebar.text-strong` | `parla_tokens(theme).sidebar_rail`, `sidebar_border`, `sidebar_hover`, `sidebar_muted`, `sidebar_text_strong` |
| `signal.default` / `signal.text` / `signal.soft` | `parla_tokens(theme).signal` / `signal_text` / `signal_soft` |
| `text.subtle` | `parla_tokens(theme).text_subtle` |
| `mention.text` | `parla_tokens(theme).mention_text` |
| `accent.soft-text` (own reactions) | `parla_tokens(theme).accent_soft_text` |
| `border.subtle` | `parla_tokens(theme).border_subtle` |
| `border.control` (field and unticked check box borders, 3:1) | `parla_tokens(theme).border_control` |
| `bg.sunken` (code block head) / `code.border` | `parla_tokens(theme).bg_sunken` / `code_border` |
| `bg.overlay` (dim behind a tour spotlight) | `parla_tokens(theme).bg_overlay` |
| `code.keyword` / `string` / `function` / `comment` / `number` / `type` (syntax) | `parla_tokens(theme).code_keyword` ... `code_type` (theme `syntax` block) |
| `success.soft` / `warning.soft` / `danger.soft` (banner and pill grounds) | `parla_tokens(theme).success_soft` / `warning_soft` / `danger_soft` |
| `presence.active` / `away` | `parla_tokens(theme).presence_active` / `presence_away` |
| `status.success` / `warning` / `danger` | `theme.status().success` / `warning` / `error` |
| logo colours | `parla_tokens(theme).logo.*` |
| `avatar.text` | `parla_tokens(theme).avatar_text` |
| `status.danger-text` / `warning-text` | `parla_tokens(theme).danger_text` / `warning_text` |

Anything not in this table does not exist in Rust yet: apply rule 3.

## Screens

The design boards the app was drawn from are the maintainer's reference and
are not published. Each board is named below; find the board for the view you
touch and match its layout, spacing and states (empty, loading, error,
offline), in light and dark. The same states are covered by the visual
baselines in `crates/ui_testing/baselines`, which every contributor can run
and read. If you cannot see the boards, follow the tokens, the rules above and
the baselines, and say so in the PR.

| App section | Boards | Code |
| --- | --- | --- |
| Sign in | Welcome, ProviderPicker, SignInBrowser, ManualEntry, WorkspacePicker, FirstSync, FirstRunTour, Stepper | `onboarding.rs`, `tour.rs`, `sign_in.rs`, `workspace_picker.rs`, `crates/parla/src/sign_in_window.rs` |
| Channel and messages | Main, Thread, MessageStates, DirectMessages | `timeline.rs`, `message_row.rs`, `thread_panel.rs`, `sidebar.rs`, `rail.rs`, `header.rs` |
| Composer and media | ComposerExtras, Popovers, CodeSnippets, VideoClip | `composer.rs`, `format_toolbar.rs`, `slash_menu.rs`, `emoji_picker.rs`, `clip.rs`, `files.rs` |
| Find and organise | QuickSwitcher, Search, Activity, SidebarOrganise, ChannelDetails, ChannelMenus, Profile | `quick_switcher.rs`, `search_panel.rs`, `conversation_menu.rs`, `profile_panel.rs` |
| Reminders and scheduled | Reminders, Scheduled, InlineEditors | `reminders_view.rs`, `scheduled_view.rs`, `send_later.rs` |
| Huddles and calls | HuddleInChannel, Huddle, CallWindows, ScreenShare | `huddle_bar.rs`, `call_bar.rs`, `call_card.rs`, `crates/calls` |
| Windows | MultiWindow, PopoutThread, MediaViewer, CodeViewer, About | `crates/parla/src/popout.rs`, `media.rs`, `about.rs`, `titlebar.rs`, `workspaces.rs` |
| Plugins | PluginRegistry, PluginInstall, PluginPanel, PluginWindow, GithubPreview | not built yet (spec plugin milestone) |
| Settings | SettingsAppearance, ThemeEditor, SettingsGeneral ... SettingsAI, SettingsMCP, McpConfirm | `crates/settings` (model), `crates/parla/src/write_confirm_window.rs` |
| States and accessibility | States, ErrorStates, A11yFocus, Localised | every view |

## Checks

- `mise run lint:design` (also an hk step, so `mise run lint` runs it): fails
  on added lines with raw colours or off-scale `px(..)`, and names the const
  to use. `--base origin/main` checks a whole branch. It cannot tell a layout
  value from an equal spacing or font value (10, 44) and does not check named
  consts or `hsla(<ident>)`, so reviewers check those by hand.
- `mise run test` runs `brand_themes_match_design_tokens` and the metrics
  test, so tokens, themes and Rust consts cannot drift.
- Look at the result: `mise run app:shot -- --fixture --appearance both`
  (skill `app-screenshot`), then compare with the board.
- UI PRs carry before and after shots (`mise run test:visual:shots`), light and dark.

## Changing the design

Design changes start in `parla.tokens.json` (and the canvas), then the themes,
then the code, in one PR, with the docs updated when behaviour changes.
