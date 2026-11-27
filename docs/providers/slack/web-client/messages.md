# messages

## Thread replies (`conversations.replies`)

Captured (first captures): form `channel`, `ts` (thread root), `limit`, `inclusive=true`,
`latest`, `cached_latest_updates`, `_x_reason=threads-view-hydration`. Parla sends
`channel`, `ts`, `limit`, `inclusive` (`true` without `oldest`, else `false`), plus
`oldest` and `cursor` when given. Response: `messages` (root first, with
`reply_count`, `latest_reply`, `reply_users`, `reply_users_count`), `has_more`,
`response_metadata.next_cursor`. Fixtures: `tests/fixtures/threads/`.

## Threaded post and broadcast (`chat.postMessage`)

Posts and replies are never resent after a timeout or 5xx, because nothing shows the server
dedupes on `client_msg_id`; only a 429 is retried.

`thread_ts` is the root; `reply_broadcast=true` is sent only for "also send to
channel" (the result has subtype `thread_broadcast`). Hand-written fixture, not
captured yet.

## Reactions (`reactions.add`, `reactions.remove`)

Form `channel`, `timestamp`, `name` (skin tone as `name::skin-tone-N`). Errors
`already_reacted` (add) and `no_reaction` (remove) are treated as success.
Hand-written from the public method docs, not captured yet.

## Custom emoji (`emoji.list`)

`emoji`: name to image URL, or `alias:<name>`. Hand-written, not captured yet.

## Thread read mark (`subscriptions.thread.mark`)

Form `channel`, `thread_ts`, `ts`. The method name and params are assumed from the
web client's `subscriptions.thread.*` family (see the `get`/`getTimestamps`
`fixtures/spike/`); verify with a capture.

## Thread follow (`subscriptions.thread.get`, `.add`, `.remove`)

Verified live from the web client in the test workspace on 2026-11-24. Following a message's
thread first reads its subscription, `subscriptions.thread.get` (form `channel`, `thread_ts`),
which answers `{ok, subscriptions: []}` for a thread the user does not follow, then calls
`subscriptions.thread.add` (form `channel`, `thread_ts`, `last_read`), answering `{ok}`.
Unfollowing calls `subscriptions.thread.remove` with the same form, answering `{ok}`.
`last_read` is a message ts: the latest reply the user has read, or the thread ts when none.

The provider sends the read cursor the store knows (the thread's newest delivered reply, the
same cursor it passes to `subscriptions.thread.mark`) without reading the subscription. With
none known it does what the web client does: reads the subscription, takes the first entry's
`last_read` (the entry shape, `channel_id`, `timestamp`, `last_read`, `latest`, is assumed from
`subscriptions.thread.getTimestamps`; the capture only showed the empty list, so
`thread_get_followed.json` is synthetic and unverified), else the thread ts. A failed read
fails a follow before `add`; an unfollow falls back to the thread ts and still removes.
`capabilities().thread_follow` is on, so the thread panel shows the bell. The follow state
itself arrives on `thread_subscribed` (see realtime.md). Fixtures:
`crates/providers/slack/api/tests/fixtures/subscriptions/thread_*.json` (hand-written; the
followed and error replies are synthetic).

## Scheduled messages (`chat.scheduleMessage`, `chat.scheduledMessages.list`, `chat.deleteScheduledMessage`)

Unverified: written from the public method docs, tested against synthetic fixtures
(`crates/providers/slack/api/tests/fixtures/scheduled/`), no capture yet.

- `chat.scheduleMessage`: form `channel`, `text`, `post_at` (unix seconds), `thread_ts`
  only for a thread reply. Response `channel`, `scheduled_message_id`, `post_at`,
  `message.text`. Errors seen in the docs: `time_too_far` (past the 120 day limit) and
  `time_in_past`; both surface with the code kept.
- `chat.scheduledMessages.list`: optional `cursor`. Response `scheduled_messages[]`
  (`id`, `channel_id`, `post_at`, `date_created`, `text`, optionally `thread_ts`) and
  `response_metadata.next_cursor` (empty means done). The provider follows every cursor
  (at most 20 pages).
- `chat.deleteScheduledMessage`: form `channel`, `scheduled_message_id`.
  `invalid_scheduled_message_id` (already sent or deleted) maps to a not-allowed error; the domain then drops the item and refetches, and an edit does not schedule a copy.
- No websocket event for scheduled changes has been captured, so none is mapped; the
  domain refetches the list after its own schedule and delete calls.

## Pins, bookmarks, saved items, Threads view and single message (UNVERIFIED)

None of these was captured, so the shapes come from the documented API and the web client's
known behaviour, the fixtures are synthetic (`crates/providers/slack/api/tests/fixtures/{pins,bookmarks,saved,inbox,messages}/`)
and the `pins`, `bookmarks`, `saved` and `thread_inbox` capabilities stay `false` until a
capture confirms the routes. A capture of pin and unpin, a bookmark add and remove, save and
unsave, opening Later and the Threads view and a permalink to an old message flips each to
`true` (or changes the client to what the capture shows).

- `pins.list`: form `channel`. Response `items[]` with `type` (`message` or `file`; files are
  skipped), `created`, `created_by`, `message.ts`.
- `pins.add`, `pins.remove`: form `channel`, `timestamp`. `already_pinned` and `no_pin` mean the
  state the caller wanted and map to success. Never resent after a timeout or 5xx; only a 429 is
  retried (same for `saved.add`/`saved.delete`, `bookmarks.remove`, `reminders.complete`/
  `reminders.delete`, `chat.deleteScheduledMessage` and `files.completeUploadExternal`).
- `bookmarks.list`: form `channel_id`. Response `bookmarks[]` with `id`, `channel_id`, `title`,
  `link`, `emoji` (`:shortcode:`); an entry without an id is dropped.
- `bookmarks.add`: form `channel_id`, `title`, `type=link`, `link`. Response `bookmark`. Never
  resent after a timeout or 5xx; only a 429 is retried. `bookmarks.remove`: form `channel_id`,
  `bookmark_id`.
- `saved.list`: form `filter=saved`, `limit`, `cursor`. Response `saved_items[]` with
  `item_type` (only `message` is kept), `item_id` (the channel), `ts`, `date_created`,
  `date_due` (`0` or `null` for none), `state` (`completed` is done) and
  `response_metadata.next_cursor`. `saved.add`, `saved.delete`: form `item_type=message`,
  `item_id`, `ts`. The classic `stars.*` family is the alternative if the capture shows it.
- `subscriptions.thread.getView` (the Threads view): form `limit`, `cursor`. Response
  `threads[]` with `root_msg` (`channel`, `ts`, `latest_reply`) and `unread_replies[]`
  (messages), `response_metadata.next_cursor`. Unread is the length of `unread_replies`,
  mentions the unread replies that name the signed-in user. The first captures'
  `subscriptions.thread.getTimestamps` (captured) only carries read cursors, no counts.
- Message flags: `pinned_to` (conversation ids; pinned when it holds the message's own
  conversation) and `is_starred` (saved).
- Single message (`message_get`): `conversations.history` with `latest=<ts>`,
  `inclusive=true`, `limit=1`; when the first message is not that `ts` (a reply is not in
  history), `conversations.replies` with `ts=<ts>`, `limit=1`, `inclusive=true` and the same
  check. `thread_not_found`, `message_not_found` and `channel_not_found` mean the message is
  gone or hidden and map to a not-allowed `message_not_found` error.

## Author names that the directory does not carry

Unverified for `bots.info`: written from the public method docs, tested against synthetic
fixtures (`crates/providers/slack/api/tests/fixtures/bots/`), no capture yet. `users.info`
is already used for single users.

- Bootstrap awaits only the first `users.list` pages and streams the rest (at most 50 pages),
  so on a large account a message author, a thread replier or a bot can be missing.
- Each `users.list` (and `users.info`) user maps `profile.title` and the user's `tz` (IANA
  name; empty strings mean unknown) onto the member, for the People directory's title,
  search and local time. Both are stored with the member.
- `USLACKBOT` (the built-in bot) arrives without `is_bot`; the provider maps it as a bot. The
  provider reports `members_partial`, since `users.list` stops at its page cap.
- Bot messages name their author themselves: `bot_profile.name`, else `username`, next to
  `bot_id`. The provider announces that once per bot as a member (`MemberUpdated`, id =
  the `bot_id`), from history, replies and live frames. This needs no call.
- Anything still unknown is looked up on demand by the domain (`Load::Members`, at most 50
  ids per request): `users.info` (form `user`) per user id, `bots.info` (form `bot`, response
  `bot.name`) per id starting with `B`, four in flight, over the shared limiter. An id that
  fails (`user_not_found`, `bot_not_found`, ...) is left out and never asked again that
  session; a rate limit or network error is not remembered.

## Rate limits and `Retry-After`

A 429 (or `ratelimited` body) carries `Retry-After`: delta-seconds or an HTTP date. Parla waits at
most 60 s whatever the server asks, reads the date form, treats a missing or unreadable value as
1 s, and holds further calls of that same method until that time is over so in-flight work does
not keep hitting the server. Slack limits are per method: a 429 on a background method (history,
users) never holds a user send.

Posts (`chat.postMessage`) are spaced per channel at one per second after a burst of five, so a
few short messages typed in a row go out at once instead of one per second. Slack documents about
one post per second per channel with short bursts allowed; the burst of five is our client's
choice within that, not a measured limit. A 429 on `chat.postMessage` holds only that channel's posts, and the outbox
resends a rate-limited post at once (the limiter's hold is the only wait), falling back to its
retry timer only after three rate limits in a row.
