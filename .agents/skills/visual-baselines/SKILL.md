---
name: visual-baselines
description: Use when adding or changing a Parla visual (golden image) scenario, regenerating baselines, or fixing a test:visual failure. How to add a FakeProvider scenario, update only the named baselines, find failures, and keep images within budget.
---

# Visual baselines

Golden PNGs live in `crates/ui_testing/baselines/<os>/components/<name>@2x-{light,dark}.png`.
Each scenario renders in light and dark (`Variant::defaults()`).

## Add a scenario

1. `crates/ui_testing/src/scenario.rs`: a `fn <name>() -> Scenario` with
   `FakeProvider` data (synthetic names only), listed in `all()`. The name is
   the baseline name: stable, unique, snake_case.
2. New data the components read: `src/source.rs` (component source traits);
   new layout or open state: `src/stage.rs` (`StageOptions`).
3. `crates/ui_testing/tests/visual_components.rs`: add the name to
   `plain_tests!(..)`, or a `#[test]` calling `run(name, opts)` for options.
4. `crates/ui_testing/scenario-map.toml`: map the source files that can change
   it, so a source change names the affected scenarios.
5. Write the baselines (below) and Read both PNGs before committing.

## Regenerate only what you meant to change

```sh
mise run test:visual:update -- <name> [<name> ..]   # sets PARLA_VISUAL_UPDATE=1
mise run test:visual -- --no-fail-fast        # list every failing scenario
```

- Never run a blanket update; `git status crates/ui_testing/baselines` must
  show only the scenarios you named. Revert others (`git checkout -- <png>`).
- A failure you did not cause (stale after a rebase): take main's PNG, then
  regenerate only your scenarios.

## Budgets and follow-ups

- Each PNG at most 200 KB, each platform dir at most 36 MB (hk checks both).
- A baseline listed in `scripts/site-shots.config.json` changed: run
  `mise run site:shots`; `mise run site:check-shots` (in `lint`) fails a stale image.
- PR: `mise run test:visual:shots` for before/after, or a `UI-Unchanged: <reason>` line.
