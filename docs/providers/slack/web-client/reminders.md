# Reminders (`reminders.add`, `reminders.list`, `reminders.complete`, `reminders.delete`)

Unverified: written from the public method docs, tested against synthetic fixtures
(`crates/providers/slack/api/tests/fixtures/reminders/`), no capture yet. Whether the web
client's session may call these routes is not confirmed, so the provider keeps
`capabilities().reminders = false`; a capture that shows the routes work with the session
flips it to `true`.

- `reminders.add`: form `text`, `time` (sent verbatim, the server parses phrases such as
  "in 20 minutes"; an unparsable one returns `cannot_parse`, kept as the error code),
  optional `user` (remind someone else). Response `reminder` with `id`, `creator`, `user`,
  `text`, `recurring`, `time`, `complete_ts`. A response with no reminder id is an error (`no_reminder_in_response`). The call is never
  resent after a timeout or 5xx (the reminder may be stored); only a 429 is retried. The
  domain reloads the list after any add error so a stored reminder shows.
- `reminders.list`: no params, no paging. Response `reminders[]` of the same shape, open and
  completed. Every field may be `null` or missing; an entry without an id is dropped.
  `time` is absent for recurring reminders, `complete_ts` is `0`, `null` or absent while
  open. `user` falls back to `creator`.
- `reminders.complete`, `reminders.delete`: form `reminder` (the id). `not_found` (already
  gone) maps to a not-allowed provider error; the domain then keeps the item removed
  (no rollback into a stale entry) and reloads the list.
- Parla's own local reminders (a provider that keeps none) are not part of these routes; the
  domain alerts them itself (see the Reminders section of the user docs).
- No websocket event for reminder changes has been captured, so none is mapped; the domain
  applies its own add, complete and delete results and reloads on open.
- Snooze, edit and "remind me about this message" have no documented route here (the public
  `reminders.add` takes no message), so they apply only to Parla's own reminders; a provider that
  keeps reminders shows neither. A capture of the web client's snooze, edit and message reminder
  would add them.
