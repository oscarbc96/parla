---
name: ship-pr
description: Use when a Parla change is ready to land - worktree from origin/main, rebase onto a moved main, gates, screenshots, PR body, signed commit, PR, squash merge and cleanup.
---

# Ship a PR

One change, one worktree, one signed squash-merged PR. Every command is a
`mise` task or plain git/gh.

## 0. Disk

```sh
df -g . | awk 'NR==2{print $4}'   # GB free; under 15: stop and report
```

## 1. Worktree

```sh
git fetch -q origin
git worktree add -b <type>/<slug> ../parla-<slug> origin/main
```

## 2. Main moved? Rebase the working tree

Only for a branch with no local commits (all work uncommitted, the normal
case here): `reset --hard` drops local commits.

```sh
git fetch -q origin
git stash -u && git reset --hard origin/main && git stash pop
```

- Conflicts: keep both sides, then `git reset -q` (unstage, keep the files).
- PNG baselines: take main's (`git checkout HEAD -- <png>`), then regenerate
  only yours (skill `visual-baselines`).
- A conflicted pop keeps the stash: drop only your own (`git stash list`,
  check the branch name, `git stash drop stash@{N}`). Never touch other stashes.

## 3. Gates (foreground)

```sh
mise run gates -- --quick --continue          # build, lint, clippy, test
mise run test:visual                          # UI change
mise run test:perf                            # hot path (domain, store, components, parla, ...)
mise run test:perf -- long_alert              # timeline/message rows: alloc_per_long_alert_redraw
```

The long_alert test prints `alloc.redraw_long_alerts measured: N`; the ceiling
is `alloc_per_long_alert_redraw` in `crates/parla/tests/perf_budget.toml`.
Over budget: fix the code; a budget changes only by a deliberate, reviewed edit.

## 4. Screenshots

UI change: `mise run test:visual:shots` renders every `FakeProvider` scenario,
light and dark, to `target/visual/<os>/shots`. Run it before and after the
change and attach both sets with the `test:visual` report. If nothing renders
differently, say `UI-Unchanged: <reason>` in the PR body instead.

## 5. Commit, PR and merge

Stage the change and commit it signed: `git commit -S`. Never commit unsigned;
if signing fails twice, stop and report.

Write the PR body from `.github/pull_request_template.md`, fill every section,
then check it:

```sh
mise run lint:pr-body -- --body-file <f> --title "<t>" --base origin/main
```

Open the PR and squash-merge it once green:

```sh
gh pr create --title "<type(scope): summary for users>" --body-file <f>
gh pr merge --squash --delete-branch
```

A section that does not apply says `N/A: <reason>`. Never merge a red PR.

## 6. Clean up

```sh
cd ../parla && rm -rf ../parla-<slug>/target
git worktree remove ../parla-<slug> && git worktree prune
git branch -D <type>/<slug>
```

## Common failures

| Symptom | Fix |
| --- | --- |
| `test:repo` times out under load | rerun it alone: `mise run test:repo` |
| `test:visual` fails after a rebase | stale baseline: regenerate only the named scenarios |
| `api.github.com` connection or 5xx error | network blip: retry the same command once |
| `gh pr merge` complains about a worktree | the merge happened; delete the local branch by hand |
