---
name: web-client-recon
description: Use when Parla needs to learn, debug or replicate a Slack web-client feature - "how does the web client do X", "what endpoint/event does Y use", "works in Slack but not in Parla", adding or changing anything in slack_api or slack_realtime (the Slack provider, codename slack), or checking whether Slack changed an endpoint. Captures the owner's own web-client traffic, redacts it, and turns it into docs/providers/slack/web-client pages, test fixtures and a failing test.
---

# Web-client recon

Every Slack endpoint and websocket event Parla uses is undocumented, so we
learn them from the owner's own web-client traffic. This skill is the one
repeatable, redacting way to do that. Slack is the provider with codename
`slack`; its code lives in `crates/providers/slack/` and its docs in
`docs/providers/slack/web-client/`.

Output of every run: a `docs/providers/slack/web-client/` page section, redacted fixtures, and
a failing test that the implementation then makes pass.

## Hard rules

- **Own session, own workspaces only.** Never capture anyone else's traffic.
- **Read-only by default.** Writes (send, react, upload, start a huddle) only
  in a private test channel or a test workspace.
- **Raw captures never enter the repo.** They live in `~/.cache/parla/recon/raw/`
  (create with mode 0700) and are deleted right after redaction. The ID map
  (`~/.cache/parla/recon/idmap.json`) is secret too and stays there.
- **Nothing is committed until `check` passes.**
- **Never paste raw tokens, cookies, IDs, names or message text** into chat,
  issues, commits or logs. Talk about shapes and counts.

## Tools

All commands are `slack_recon` subcommands (Rust, `crates/providers/slack/recon`:
Slack's rules from `slack_redact` and read-only endpoint list plugged into the
provider-agnostic `recon` crate), run through `mise`:

```sh
mise run recon:redact -- <raw.har> --out <dir>   # redact + split per exchange/frame
mise run recon:check -- <dir>                    # fail on any leftover secret/ID/PII
mise run recon:summary -- <dir>                  # methods/events + response keys
mise run recon:extract -- <dir> --area <name> [--fixtures <dest>]  # draft web-client docs section; copy + re-check fixtures
mise run recon:diff -- <browser-dir> <parla-dir>  # (M1) where Parla's requests differ
mise run recon:verify                            # (M1) read-only drift check
```

The ID map (`~/.cache/parla/recon/idmap.json`) keeps synthetic IDs stable
across captures; it is secret and never leaves that folder.

## Workflow

1. **Pick one feature.** Name it and the target conversation (test channel
   for anything that writes). Check `docs/providers/slack/web-client/catalogue.md` first; if
   it is already documented and verified recently, skip to step 6.
2. **Capture.**
   - HTTP: use a browser automation tool on the owner's signed-in
     `app.slack.com`, or the owner by hand. Open DevTools Network, filter
     `api/`, clear, perform the feature once, stop.
   - Websocket frames (typing, presence, huddle events, message events):
     DevTools "Save all as HAR with content" includes websocket messages.
   - Save the HAR to `~/.cache/parla/recon/raw/<date>-<feature>.har`.
3. **Redact:** `recon:redact` into a scratch dir outside the repo, then delete the
   raw HAR.
4. **Check:** `recon:check <dir>` must print `clean`. If not, fix the redactor
   (add the pattern to `slack_redact`), never hand-edit fixtures to hide a leak.
5. **Extract:** `recon:summary <dir>` lists methods/events and their response keys.
   `recon:extract <dir> --area <page>` prints a draft section (params, response
   shape, enums) to paste into the area page. Copy only the exchanges the
   feature needs (`--fixtures <dest>` copies and re-checks them) into
   `crates/providers/slack/api/tests/fixtures/<feature>/` (HTTP) or
   `crates/providers/slack/realtime/tests/fixtures/<feature>/` (frames). Run `recon:check` on the
   destination too.
6. **Document:** add or update the area page in `docs/providers/slack/web-client/`
   (`messages.md`, `huddles.md`, ...): method or event name, request params
   the web client sends (and which are required), response shape (field,
   type, optional?, enums seen), errors seen, rate-limit tier if observed,
   and the fixture path. Update `catalogue.md`: feature, method/event,
   fixture, first seen, last verified (today).
7. **Test first:** write a test that replays the fixture (wiremock for HTTP,
   the in-process fake websocket for realtime) and asserts Parla sends the
   same params the web client sent. Run it and see it fail. Only then
   implement in `slack_api`/`slack_realtime` until it passes.
8. **Compare (debugging "works in Slack, not in Parla"):** record Parla's own
   traffic with `PARLA_RECORD=<dir>` (debug builds), redact both, and `recon:diff` the
   two captures: missing params, different call order, different headers.
9. **Commit** fixtures, docs and test together (feature PR per the project's
   workflow), after `check` and `mise run test` are green.

## Drift check

Before each milestone, or when sign-in or realtime breaks: re-call only the
read-only methods in `catalogue.md` with the owner's session, at low rate,
and compare response shapes with the fixtures. Added fields are notes;
removed or retyped fields are failures to fix. Never re-call write methods.

## Done when

- `recon:check` is clean on every committed fixture.
- `docs/providers/slack/web-client/` and `catalogue.md` describe the feature.
- A replay test exists, failed before the implementation, and passes now.
