# realtime

Session websocket of the web client. The connection recipe is under
"Connection" below. Fixtures: `crates/providers/slack/realtime/tests/fixtures/spike/`
(`ws-<type>.json` received, `ws-send-<type>.json` sent).

### Connection (ws)

Kind: ws. `wss://wss-primary.slack.com/?token=...&sync_desync=1&slack_client=desktop&start_args=...&no_query_on_subscribe=1&flannel=3&lazy_channels=1&gateway_server=<TEAM_ID>-1&batch_presence_aware=1`
(plus a leading `frt=` on reconnect). No `hello` after a reconnect; no
`permessage-deflate`. Handshake headers: unconfirmed (probe).

### ping / pong (ws)

Sent `{"id":N,"type":"ping"}`; received `{"reply_to":N,"type":"pong"}`.
Fixtures: `ws-send-ping.json`, `ws-pong.json`.

### reconnect_url (ws)

`{"type":"reconnect_url","url":"wss://...?frt=...&token=..."}`; the URL is a
credential. Fixture: `ws-reconnect_url.json`.

Parla keeps the latest one and tries it once on the next reconnect. A stale one
(for example after a long sleep) can be refused with HTTP 401; that is not a sign
the session ended, so Parla falls back to a fresh connect URL (a 401 on a fresh URL
still means the session expired). The reconnect backoff restarts at its minimum
only after a connection stayed up for 30 s, so a server that sends `hello` and
hangs up each time keeps backing off.

A resumed connection that sends `hello` (and a new `reconnect_url`) but then
delivers nothing, so it dies by pong timeout (about 71 s: 60 s of silence plus a
10 s pong wait), is a dead session; resuming it again lands in the same place.
Parla discards the resume URL, reconnects fresh through the connect URL fetch
after the normal backoff, and does not restart the backoff for that connection.
A resumed connection that delivered real frames keeps its latest `reconnect_url`.

A queued send waits for the normal backoff reconnect; only the user's "Retry now"
skips the wait, and even then never starts two attempts less than 5 s apart. A
connect that hangs is dropped after 20 s. Every drop is logged as one line
(`realtime lost: <cause> (fresh|resume, up Ns|never connected)`) with the cause
(`server goodbye`, `closed by server`, `closed by server (code)`, `connection
closed`, `connection error (kind)`, `write failed`, `pong timeout`, `connect failed
(kind)`, `connect timeout`, `handshake rejected: http N`, `reconnect url rejected`,
`url fetch failed (kind)`); no URL, token, id or message text.

### message (ws)

Subtypes seen: `message_changed`, `message_deleted`, `message_replied`,
`thread_broadcast`, `huddle_thread`. Fixtures: `ws-message-<subtype>.json`.
A subtype-less new message is not yet captured.

`message_changed` carries the whole edited message (`message`: text, blocks,
attachments, files, reactions; `edited` only when a person edited it). Parla maps
it to a full message replacement, so link unfurls and bot or file updates show
without an "edited" mark.

### user_typing (ws, sent and received)

Sent: `{"id":N,"type":"user_typing","channel":"<id>"}`. Fixture:
`ws-send-user_typing.json`. Parla sends it from `ChatProvider::typing` while the
user types in a draft, at most once every 3 s per conversation (the throttle is
in `domain`), and drops it while the socket is down. The throttle is per
conversation, not per thread: a thread signal within 3 s of a channel signal
in the same conversation (or the reverse) is dropped. For a thread reply draft
Parla adds `"thread_ts":"<root ts>"`: **needs recon confirmation** (no capture
shows the field on a sent frame).

Received: `{"type":"user_typing","channel":"<id>","user":"<id>","thread_ts":"<ts>"?}`,
mapped to `ProviderEvent::Typing`. **Needs recon confirmation**: incoming typing
from others was not observed in the first captures (a two-user check: one
account types while the other runs Parla). Parla shows a typer for 6 s after
their last frame, or until their message arrives.

### presence_sub (ws, sent) and presence_change (ws)

`{"id":N,"type":"presence_sub","ids":[...]}`; reply events
`{"type":"presence_change","presence":"active"|"away","users":[...]}`.
Fixtures: `ws-send-presence_sub.json`, `ws-presence_change.json`.

Parla subscribes only to the people on screen (the peers of the direct-message
rows shown, the open direct message and the open profile): the set is replaced as
a whole through `ChatProvider::watch_presence`, debounced by the app (300 ms) and
resent after a reconnect. Ids are chunked at 750 per frame. There is no
unsubscribe frame in the captures, so a person who leaves the screen simply stops
being shown. After each subscription Parla also sends
`{"id":N,"type":"presence_query","ids":[...]}` because the connection URL carries
`no_query_on_subscribe=1`; that frame was never seen in a capture (unverified),
so a person's first dot may wait for their next `presence_change`.

### subscription_update (ws, both ways)

Subtypes `user_change_subscribe_request` / `_response`,
`dnd_invalidated_subscribe_request` / `_response`. Fixtures:
`ws-send-subscription_update.json`, `ws-subscription_update.json`.

### tickle (ws, sent)

`{"id":N,"type":"tickle","reason":"focus"|"mousedown"}`. Fixture:
`ws-send-tickle.json`.

### Reactions, read marks, files and emoji (ws)

Parsed into typed variants. `thread_marked` is captured
(`ws-thread_marked.json`: `subscription.{channel,thread_ts,last_read}`).
`message_replied` carries the root with updated reply fields.
`reaction_added`/`reaction_removed` (`item.{type,channel,ts}`; file items are
ignored), `im_marked`/`group_marked`/`mpim_marked` (same shape as
`channel_marked`), `file_shared`, `file_change`, `file_deleted` and
`emoji_changed` are NOT captured yet: their fixtures in
`crates/providers/slack/realtime/tests/fixtures/events/` are hand-written
from the public event docs.

### People events (ws)

`dnd_updated`, `dnd_updated_user` (`user`, `dnd_status`) and `pref_change`
(`name`, `value`) are typed; see [people](people.md). `user_change` also carries
`profile.status_text`, `status_emoji` and `status_expiration`. Hand-written fixtures in
`crates/providers/slack/realtime/tests/fixtures/events/` (`dnd_*`, `pref_change_*`,
`user_change_status`), not captured yet; the first captures' `ws-pref_change.json` has a boolean
`value`.

### Other events seen (ws)

`channel_marked`, `thread_marked`, `thread_subscribed`,
`update_global_thread_state`, `user_change`, `user_invalidated`,
`visits_changed`, `activity`, `badge_counts_updated`, `pref_change`,
`user_interaction_changed`, `draft_create`, `draft_update`. One fixture each,
`ws-<type>.json`.

### Pins, bookmarks, saved and Threads events (ws)

`thread_subscribed` and `update_global_thread_state` were seen in the first captures and now mark the
Threads view stale (`ThreadInboxChanged`); `thread_marked` also does. `thread_subscribed`
(`ws-thread_subscribed.json`: `subscription.{channel,thread_ts,active}`) also reports whether
the user follows that thread (`ThreadFollowChanged`, `followed = active`; a missing `active`
reads as followed). No unsubscribe frame has been captured. The rest is UNVERIFIED
(no capture, synthetic fixtures `crates/providers/slack/realtime/tests/fixtures/events/`):
`pin_added` and `pin_removed` (`item.type` `message`, `item.channel` or `channel_id`,
`item.message.ts`; a file pin is `Unknown`) map to `PinChanged`; `bookmark_added`,
`bookmark_changed`, `bookmark_removed` (`channel_id`) to `BookmarksChanged`; `star_added` and
`star_removed` on a message item to `SavedChanged`.

### Team rename (ws)

`team_rename` (`{"type":"team_rename","name":"..."}`) updates the workspace name.
Not verified (public docs shape, synthetic fixture
`crates/providers/slack/realtime/tests/fixtures/events/team_rename.json`).
