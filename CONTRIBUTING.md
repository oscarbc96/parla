# Contributing to Parla

Not created by, affiliated with, or supported by Slack Technologies, LLC.

Read `AGENTS.md` first. It has the project rules in short form, and
`docs/design/` holds the design system.

## CLA

Contributions need a signed CLA (`CLA.md`). CLA Assistant asks you to sign once on
your first pull request. Code under another licence must keep its header and be listed in `NOTICE`.
Never copy code from GPL-licensed projects such as Zed.

## Workflow: local green is the gate

1. `mise install` then `hk install --mise` (once per clone).
2. Work on a short-lived branch, test first (write the failing test, see it fail,
   make it pass).
3. `mise run build`, `mise run lint` and `mise run test` must pass locally;
   `mise run gates` runs the whole list (also clippy, visual, perf tests, i18n,
   site translations, repo tests) and writes `target/gates/summary.md` for the
   PR (`--quick` for the first four). For hot paths run `mise run perf:suite`
   and paste `target/perf/suite.md` into the Performance section;
   `mise run perf:ab` measures the merge base and your branch with the same
   arguments and writes a before/after table to `target/perf/ab.md` (flags
   more than 10% worse or over budget). `mise run test:affected` runs nextest
   for only the changed crates and their dependents. Set `PARLA_SCCACHE=1`
   (with `sccache` installed) to opt in to sccache as the rustc wrapper; it
   is off by default. `mise run check:windows` compile-checks and lints the
   Windows-only code (`settings`, `mcp`) for `x86_64-pc-windows-msvc` from any
   host; run it when you touch `cfg(windows)` code. The hk
   pre-commit and pre-push hooks enforce this.
   Tools are pinned to exact versions in `mise.toml`, and Renovate proposes
   their bumps. Before a push, a check compares your commits and diff with a
   private list of terms kept outside the repo (`$PARLA_DENYLIST` or
   `~/.config/parla/denylist.txt`, one term per line); it prints counts only
   and is skipped when no list exists.
4. Open a pull request with `.github/pull_request_template.md` filled in
   (the `pr-template` check enforces it; `mise run lint:pr-body` runs it
   locally): summary, plan task, type, screenshots and the visual-diff
   report for UI changes, perf numbers for hot paths,
   tests, docs, checks run, security. Outside
   contributors are merged after the maintainer's review; the maintainer
   self-merges features once they are green locally (squash merge, signed
   commits).
5. CI re-runs the checks on macOS and Linux; `mise run check:windows` covers
   the Windows-only code locally. A red run is fixed forward.

Small fixes and docs may go straight to `main` for the maintainer.

Debugging what the app shows: `mise run app:shot -- --fixture` launches Parla on
the synthetic fixture and saves a PNG of its window under `target/shots/`
(macOS; see `.agents/skills/app-screenshot/SKILL.md`). Screenshots of a real
account may hold private data: keep them local, never commit or attach them.

## Translations

Parla speaks English (the source), Spanish and Catalan, in the app and on the
docs site. The owner and contributors who read the language write the
translations. Every change that adds or edits user-facing text updates every
locale in the same PR: an English message in a crate's `i18n/en.ftl` also goes
into `es.ftl` and `ca.ftl`, and an English docs page also goes into its
`*.es.mdx` and `*.ca.mdx` files. Use `t!` in code, never a literal.

A machine-drafted translation is allowed only while it is marked
`i18n-draft` (`# i18n-draft` before the Fluent message, `// i18n-draft` in a
site string table, `{/* i18n-draft */}` in MDX). Drafts live on a branch until
someone who reads the language reviews them and removes the marker; the check
fails while any draft is left, so drafts never reach `main` or a release.

To add a locale, with `<code>` a language tag such as `fr`:

1. Add the entry to `i18n/locales.toml` (the one list of locales) and mirror it
   in `site/src/i18n/config.json`.
2. Add `crates/**/i18n/<code>.ftl` next to every `en.ftl` (core and provider
   crates).
3. Add `crates/parla/macos/<code>.lproj/InfoPlist.strings`.
4. Add `site/src/i18n/<code>.ts` with the same keys as `en.ts`.
5. Add `site/content/docs/*.<code>.mdx` for every English page and
   `meta.<code>.json`.
6. Run `mise run i18n:check` and `mise run site:check-translations`. They list
   any file still missing; nothing else names a locale.

Run `mise run i18n:check` before every commit that touches text. The hk
pre-commit hook and CI run it too.

## UI tests

Three layers, all on `FakeProvider`, none needing a display:

- View tests: plain unit tests on components.
- Visual (golden image): `mise run test:visual` renders offscreen and compares
  with the PNG baselines in `crates/ui_testing/baselines/<os>/`.
- End to end: `mise run test:e2e`.

A failing visual test writes `target/visual/<os>/` (expected, actual and diff
images plus `report.md` and `report.html`). CI stores no artifacts: the `ui` job prints the
report into its job summary. Run `mise run test:visual:update` only when
the change is meant to alter pixels, then review the PNG diff before
committing. CI covers macOS for now; other platforms' baselines are added once
their capture works.

For PR evidence, run `mise run test:visual:shots` before and after the change:
it renders every `FakeProvider` scenario, light and dark, to
`target/visual/<os>/shots`. Attach those images and the `test:visual` report;
GitHub shows the diff of changed baselines. If nothing renders differently, add
a `UI-Unchanged: <reason>` line. Write the PR body from the template and check
it with `mise run lint:pr-body -- --body-file <file> --title "<title>" --base
origin/main`.

The site's screenshots (`site/public/shots`) are rendered from baselines listed
in `scripts/site-shots.config.json`: run `mise run site:shots` when one of them
changes (`mise run site:check-shots`, part of `mise run lint`, fails otherwise).

## Releases

Release builds sign with the project's self-signed certificate on a version
tag, and ad hoc when its secrets are absent. Notarisation needs the paid Apple
Developer Program, which the project does not have, so macOS does not trust the
signature: users approve the first open. On macOS 15 or later that means opening
Parla once, then System Settings > Privacy & Security, scrolling to Security and
clicking **Open Anyway** next to the Parla message, then confirming and entering
the password. On macOS 14, Control-click (right-click) Parla in Applications and
choose **Open**, then **Open** again. It is only needed the first time.

The owner creates the self-signed certificate once with
`mise run release:self-signed-cert -- <dir outside the repo>` and sets the
`MACOS_SELF_SIGNED_P12` and `MACOS_SELF_SIGNED_PASSWORD` secrets it prints in
the tag-only `release` environment. The build job bakes `PARLA_UPDATE_PUBLIC_KEY`
into the app, so it is a repository variable:

```sh
gh variable set PARLA_UPDATE_PUBLIC_KEY --body "<public key>"
```

`PARLA_UPDATE_SECRET_KEY` stays a secret of the `release` environment, read by
the tag-only `sign` job:

```sh
gh secret set PARLA_UPDATE_SECRET_KEY --env release
```

Together they sign and verify the in-app update.

## Rules

- No Slack branding, logos, colours, icons, fonts, sounds or screenshots. No
  "slack" in crate, binary, repo or domain names.
- Never commit captured traffic, tokens, cookies or real user data. Use the
  redaction tooling and the `web-client-recon` skill.
- JavaScript tooling is Bun only (its built-in formatter and type checker).
- Commits are signed. Use Conventional Commit messages (`feat:`, `fix:`, `docs:`,
  `chore:`).

By participating you agree to the `CODE_OF_CONDUCT.md`.
