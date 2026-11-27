# boot

Not yet captured: neither 2026-11-13 capture reloaded the page, so
`client.userBoot` and `client.shouldReload` are still unseen. What the captures
do show is the request envelope and the post-boot count call. See
[realtime.md](realtime.md).

### Request envelope (http)

URL `https://<workspace>.slack.com/api/<method>?_x_id=<uuid8>-<epoch>.<n>&_x_gantry=true&_x_desktop_ia=4&_x_csid=<id>&slack_route=<TEAM_ID>&_x_version_ts=<build>&_x_foreground=true&_x_frontend_build_type=<build>&fp=<hex>&_x_num_retries=0`.
Form body: `token`, the method params, then `_x_reason`, `_x_mode=online`,
`_x_app_name=client`, `_x_sonic=true`. `_x_version_ts` is the web build
timestamp (also the websocket `agent_version`); `slack_route` is the team id.

### client.counts

Kind: http, read-only. Params: `thread_counts_by_channel`, `org_wide_aware`,
`include_file_channels`, `include_all_unreads`, `dry_run_last_fetched`.
Response keys: `activity_v2`, `alerts`, `channel_badges`, `channels[]`, `ims[]`,
`mpims[]` (each `id`, `has_unreads`, `last_read`, `latest`, `mention_count`,
`updated`, `history_invalid`), `threads`, `saved`, `file_channels`,
`counts_last_fetched`, `ok`. Fixture: `client.counts.json`.

### Other calls seen after boot

`visits.list`, `visits.record`, `users.interactions.list`,
`subscriptions.thread.get`, `subscriptions.thread.getTimestamps`,
`conversations.history`, `conversations.replies`, `conversations.mark`,
`messages.list`, `users.prefs.set`, `users.profile.set`, `drafts.create`.
Fixtures with the method name in `crates/providers/slack/api/tests/fixtures/spike/`.

### client.userBoot conversation entries (the maintainer's own boot, 2026-11-14)

Observed keys, no content kept. `ims[]` carry `is_open` (a boolean) and `user`.
Group DMs (`channels[]` entries with `is_mpim`) carry `members`, an array with
the ids of every member including the signed-in user, and no `is_open`; they
also carry the internal `name` (`mpdm-<handle>--<handle>-<n>`), whose handles
can be empty or differ from the user's current `name`, so group labels resolve
by `members` ids. `client.counts` lists only a few `mpims[]` (the ones with
recent state), so absence there does not mean closed. No documented read-only
source of group DM open state was found, so openness is inferred (see
[conversations.md](conversations.md)): the group DMs `client.counts` lists with a
latest message in the last 30 days, or with unreads, are open; the rest are hidden.
