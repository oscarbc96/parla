# Slack web-client notes

Everything Parla knows about the undocumented web-client endpoints and
websocket events, learned from the maintainer's own redacted traffic. Not affiliated
with Slack Technologies, LLC.

One page per area: [auth](auth.md), [boot](boot.md),
[conversations](conversations.md), [messages](messages.md), [people](people.md),
[realtime](realtime.md), [huddles](huddles.md), [files](files.md). The index of
every method and event is the [catalogue](catalogue.md).

## Page format

One `###` section per method or event, drafted by `recon extract` and then
edited by hand:

- Kind (`http` or `ws`) and the request params the web client sends, noting
  which are required.
- Response shape as a table: field path, type, optional, enum values seen
  (only for `type`, `subtype`, `error`).
- Errors seen, rate-limit tier if observed, and the fixture path.

Update the catalogue row in the same change. All fixtures and pages must pass
`mise run recon:check -- docs/providers`. The check also fails on any URL whose host is
not a placeholder (`example.com`, `example.test`, ...) and on long free-text fields
(title, footer, fallback, ...) that are not lorem ipsum. The redactor rewrites both.

## How to add a section

Follow the `web-client-recon` skill (`.agents/skills/web-client-recon/SKILL.md`).
