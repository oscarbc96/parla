# Parla: instructions for coding agents

Parla is an open-source, low-resource desktop Slack client in Rust on GPUI.
Not created by, affiliated with, or supported by Slack Technologies, LLC.

## Read first

- Design system: `docs/design/` (tokens are the source of truth). Any UI
  work loads the `design-system` skill first.
- Do the task you were given; do not expand scope.

## Workflow

- Local green is the gate: `mise run build`, `mise run lint`, `mise run test`
  must pass before you commit. Install hooks once with `hk install --mise`.
  `mise run gates` runs all of them plus clippy, visual, perf tests, i18n,
  site translations and repo tests in order (`--quick`: build, lint, clippy,
  test; `--only`, `--skip`, `--continue`) and writes `target/gates/summary.md`
  for the PR's Tests section.
- Features land through short-lived branches and self-merged, squash-merged PRs.
  Small fixes and docs may go straight to `main`.
- Commits are signed. If signing fails, stop and report; never commit unsigned.
- Docs snapshots are frozen: edit `site/content/docs`, never
  the release snapshots written by `mise run site:docs-version` (under
  `site/content/versions/`) or `versions.json`, except through that task.
- Test first: write the failing test, see it fail, then make it pass.
- Everything you run is a `mise` task; do not invent ad hoc commands.
- Every change is documented in the same commit or PR. User-visible behaviour
  goes in `site/content/docs/`, in English (`*.mdx`) and every existing
  translation (`*.<locale>.mdx`, today `es` and `ca`), in the same change (also
  `meta.json` and `src/i18n`; `mise run site:check-translations` and CI
  enforce it). App text is documented the same way: an English `.ftl`
  change (`crates/**/i18n/en.ftl`) updates every locale in the same change
  and `mise run i18n:check` must pass (CONTRIBUTING.md "Translations").
  For a typo or formatting-only English edit add a
  `Translations-Unchanged: <reason>` line to the commit message or PR body;
  behaviour changes update the docs; provider
  endpoint or event changes update `docs/providers/<codename>/`. Pure
  refactors, test-only and CI-only changes are exempt; say so in the PR.
- Site changes: run `bun run format` in `site/` before committing. Site
  tasks run through Turborepo (`site/turbo.json`, `mise run site:*`); its local
  cache `site/.turbo/cache` is capped at 14 days / 500 MB (`cacheMaxAge`,
  `cacheMaxSize`), telemetry is off, and the Pages build uses `--force`.
- PRs use `.github/pull_request_template.md` and fill every section; the
  `pr-template` CI check fails otherwise. Check the body before opening
  the PR with `mise run lint:pr-body -- --body-file <file> --title
  "<title>" --base origin/main`. Rules: no section empty; a section that
  does not apply says `N/A: <reason>` (bare `N/A` fails); every box is
  ticked or its line ends with `N/A: <reason>`; the ticked type matches
  the title; security boxes are always ticked.
- UI code follows the design system (skill `design-system`): colours from
  the theme, sizes from `themes::metrics`, no raw `rgb`/`hsla`/`#hex` or
  off-scale `px(..)` (`mise run lint:design` fails them), a missing token is
  added to `docs/design/parla.tokens.json` and the themes first, and the
  `design-reviewer` subagent checks UI PRs.
- UI changes (components, themes, icons, `parla` views, visual baselines,
  `site/src`) put before and after screenshots per changed platform, in
  light and dark, from `FakeProvider` scenarios (`mise run test:visual:shots`
  renders them), and the `mise run test:visual` report (attach its images or
  summary) in the PR. If nothing renders differently, add a
  `UI-Unchanged: <reason>` line.
- Site screenshots (`site/public/shots`) come from visual baselines:
  `mise run site:shots` after a baseline listed in
  `scripts/site-shots.config.json` changes; `mise run site:check-shots`
  (part of `mise run lint`) fails on a stale image.
- Performance budgets are hard limits. Changes to hot paths
  (`domain`, `store`, `store_sqlite`, `components`, `parla`, `calls`,
  `i18n`, provider realtime, markup and provider source, the perf budget
  file) run `mise run test:perf` and `mise run perf:suite` (3 runs of idle,
  scroll, scroll-input; waits for a quiet machine; `target/perf/suite.md`;
  `mise run perf:ab` adds a merge-base comparison in `target/perf/ab.md`) or
  the affected `mise run perf:measure` scenarios and put the numbers in the PR's Performance
  section, or a `Perf-Unaffected: <reason>` line. Over budget: fix it;
  budgets change only with a deliberate, reviewed edit. No polling, no idle
  timers, no unbounded caches.
- `mise run test:affected` runs nextest for only the crates changed versus
  `origin/main` and their reverse dependents (a quick loop; `mise run test`
  stays the gate). `PARLA_SCCACHE=1` opts in to sccache when installed.
- Repo tooling scripts (`scripts/*.ts`) are Bun, tested with `mise run
  test:repo`.
- PR title becomes a release-note line; write it for users. It is a
  Conventional Commit, `type(scope): summary`, with type `feat`, `fix`,
  `perf`, `docs`, `refactor`, `test`, `ci`, `build`, `chore` or `revert`
  (`!` before the colon for a breaking change). `docs` is for user docs;
  research changes use `chore(research)`, ...
  Direct commits to `main` follow the same rule. release-please turns
  these into `CHANGELOG.md` and GitHub Releases.
- Renovate (`renovate.json`: cargo in both workspaces, github-actions, bun in
  `site/`, mise tools) opens PRs on Monday mornings titled `chore(deps)` or
  `ci(deps)`, once a release is 3 days old. Minor and patch bumps arrive
  grouped per ecosystem, majors alone, and crates that must move together
  (gpui, fonts, wasmtime, wit) share one PR. wasmtime stays on the 48 LTS line
  (patches only); `rust`, `bun` and `hk` are bumped by hand. Its Dependency
  Dashboard issue lists what is pending. They follow the same gate as any PR:
  local green (`mise run build`, `lint`, `test`) before the squash merge, plus
  `mise exec -- cargo deny check` for cargo bumps and `bun run format` in
  `site/` for bun bumps. `mise run lint:renovate` validates the config.
- Debugging app or UI behaviour: capture and look at the real window with
  `mise run app:shot` (skill `app-screenshot`) before and after the fix, and
  Read the PNG. Real-account shots stay under `target/` and are never committed
  or uploaded; PR screenshots come from `FakeProvider` fixtures (`--fixture`).

## Conventions

- Crates are unprefixed snake_case under `crates/` (`settings`, `themes`,
  `domain`, ...); the binary crate is `parla`. Only globally named things keep
  the `parla_` prefix (plugin API/SDK, `PARLA_*` env vars, `parla://`).
- Provider-agnostic: the core (`domain`, `store`, `components`, `settings`,
  `redact`, `recon`, ...) is neutral; Slack is only the first provider.
  Provider code lives under `crates/providers/<codename>/` in crates named
  `<codename>_*`; Slack's codename is `slack` (`slack_api`,
  `slack_realtime`, `slack_auth`, ...). Core never depends on a provider
  crate; only the `parla` binary wires providers through its registry. No
  provider types or names in core code, tests or paths: core tests use
  `domain`'s `FakeProvider`, and provider tests, fixtures and docs stay under
  `crates/providers/<codename>/` and `docs/providers/<codename>/`.
- Licence `MPL-2.0`; docs are CC-BY-4.0.
- JavaScript: Bun only (its built-in formatter and type checker). No Node, npm or pnpm.
- No "slack" (or look-alike) in the product, binary, repo, domain or app names
  (the Slack provider crates under `crates/providers/slack/` are the one
  exception). No Slack logos, colours, icons, fonts, sounds or screenshots anywhere.
- Never commit tokens, cookies or unredacted captures.

## Dev notes

- Debug builds keep credentials and store keys in
  `<data_dir>/development_credentials.json` (mode 0600, unencrypted), not the
  keychain, so rebuilds do not prompt. Release builds always use
  the keychain. `PARLA_DEV_USE_KEYCHAIN=1` forces the keychain in a debug build
  (pair it with `mise run dev:cert`). An existing keychain session is not
  copied; sign in again. Never log or commit that file.

## Agent tooling

- Skills live in `.agents/skills/<name>/SKILL.md`; subagents in
  `.agents/agents/<name>.md` (see `.agents/agents/README.md`).
- `CLAUDE.md`, `.claude/skills` and `.claude/agents` are symlinks. Never put
  real files in vendor paths; an hk check enforces this.
- Use the `design-system` skill before any UI change.
- Use the `app-screenshot` skill to see the running Parla window (macOS).
- Use the `web-client-recon` skill to learn or change anything about Slack's web
  client endpoints or events.
- Use the `ship-pr` skill to land a change (gates, PR body, signed commit, merge).
- Use the `gpui-pitfalls` skill when writing or reviewing GPUI code.
- Use the `visual-baselines` skill to add scenarios or update baselines.
- The `implementer` subagent carries the standing rules for implementation briefs.
