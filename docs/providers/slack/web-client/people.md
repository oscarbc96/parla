# people

Profiles, own status and presence, do not disturb and notification preferences. All HTTP
endpoints on this page are **unverified**: written from the public method docs and the
plan, tested against synthetic fixtures (`crates/providers/slack/api/tests/fixtures/{people,dnd,prefs}/`,
`crates/providers/slack/realtime/tests/fixtures/events/`), no capture yet. Only
`users.prefs.set` (name/value form) and the `user_change` and `pref_change` frames come
from the first captures. Every response field tolerates `null` and absence; entries with an empty
id are dropped.

## Profile (`users.profile.get`, `users.info`)

One profile is two calls joined: `users.profile.get` (form `user`, `include_labels=true`)
for `display_name`, `real_name`, `title`, `pronouns`, `email`, `phone`, `status_text`,
`status_emoji`, `status_expiration`, `image_192` (then `image_512`, `image_72`, `image_48`;
https only) and `fields` (an object keyed by field id of `{value, alt, label}`, or `[]`
when empty); `users.info` (form `user`) for `tz`, `is_bot` and `deleted`. Custom fields
with an empty label or value are dropped and the rest sorted by label. Error
`user_not_found` / `user_not_visible` maps to `NotAllowed`.

## Profile pictures

A user (boot, `users.list`, `users.info`, search) carries `profile.image_48`, `image_72`,
`image_192` and `image_512`. A member's picture, shown in list rows (36 px, so 72 px at 2x), is
the first https one of `image_72`, `image_48`, `image_192`, `image_512`; the profile card
uses `image_192` (see above). A bot's is `bot.icons.image_72`, then `image_48` (`bots.info`,
unverified like the rest of that call). A URL on one of the provider's own file hosts (the same
check as file downloads, `is_file_host`) is fetched through the session; any other https URL
(a gravatar) is fetched without credentials; a non-https URL is dropped.
Members with no picture show initials.

## Own status (`users.profile.set`)

Form `profile` = JSON `{"status_text", "status_emoji", "status_expiration"}`. Clearing
sends empty text and emoji and `0`. The emoji is `:shortcode:` with `::skin-tone-N` when a
tone is set. Errors `profile_status_set_failed_not_valid_emoji` and
`profile_status_set_failed_not_valid_text` map to `NotAllowed`. The `user_change` frame
carries the new `profile.status_*` and maps to a member update plus a profile change.

## Presence (`users.setPresence`)

Form `presence=auto|away`.

## Do not disturb (`dnd.info`, `dnd.teamInfo`, `dnd.setSnooze`, `dnd.endSnooze`)

- `dnd.info` (no params, own state): `dnd_enabled`, `next_dnd_start_ts`,
  `next_dnd_end_ts`, `snooze_enabled`, `snooze_endtime`, `snooze_remaining`.
- `dnd.teamInfo`: form `users` (comma list, at most 50 per call; larger asks are split,
  repeated and empty ids are not sent, and the provider refuses more than 500 in one ask).
  Response `users`: id to `{dnd_enabled, next_dnd_start_ts, next_dnd_end_ts}`.
- `dnd.setSnooze`: form `num_minutes`. The answer carries only the snooze fields, so the
  provider reads `dnd.info` afterwards for the schedule.
- `dnd.endSnooze`: no params. `snooze_not_active` counts as success (already over).
- Frames `dnd_updated` (own; `user` may be absent and is then the signed-in user) and
  `dnd_updated_user` carry `dnd_status`; both map to `DndChanged`.

## Notification preferences (`users.prefs.get`, `users.prefs.set`)

`users.prefs.get` returns `prefs.all_notifications_prefs` (a JSON string) and
`prefs.muted_channels` (comma list). Assumed shape of the JSON: a `global` object and
per-conversation objects `{desktop, mobile, muted, ...}` either under `channels` or at the
top level; both are read. `desktop` `everything` / `mentions` / `nothing` maps to the
level, anything else to the default; a conversation is muted when its entry says so or it
is in `muted_channels`.

`users.prefs.set` (form `name`, `value`, shape from the first captures) is a
read-modify-write: Parla reads the current prefs, changes only that conversation's entry
(`desktop` set to the level, or removed for the default; `muted`; `mobile` is left
untouched) keeping
every other key, writes `all_notifications_prefs`, and writes `muted_channels` when the
mute changed. Two writes are not atomic: a failure after the first leaves the second
undone and the domain refetches. Nothing is written unless `all_notifications_prefs` parsed as
a JSON object and `muted_channels` is present (a null or garbled read is an error, not an
empty start), and the provider runs one such write at a time. The `notify_prefs` capability
stays off until a capture confirms this shape.

The `pref_change` frame (`name`, `value`) for `all_notifications_prefs` or
`muted_channels` queues one coalesced `users.prefs.get` and a `NotifyPrefsChanged`
event; other names are ignored.

## User groups (`usergroups.list`)

**Unverified, needs recon confirmation**: written from the public method, not from a
capture of the web client's own call. Tested against the synthetic fixture
`crates/providers/slack/api/tests/fixtures/usergroups/list.json`. Form
`include_count=true`, `include_users=true`, `include_disabled=false`; read once per
bootstrap (tier 2, background). Each entry of `usergroups` gives `id`, `handle`, `name`,
`user_count` (a number, or a numeric string) and `users` (member ids). Entries with an
empty `id` or `handle`, or a non-zero `date_delete`, are dropped. The provider maps each to
a neutral user group with `is_member` set when `users` holds the signed-in user, and the
domain keeps at most `MAX_USER_GROUPS` per workspace. A group mention is written
`<!subteam^ID>` and read from `<!subteam^ID|@handle>` or a rich-text `usergroup` element.
The `subteam_created`, `subteam_updated` and `subteam_members_changed` frames are not
handled yet: groups refresh on the next bootstrap.

Verify every item above with a capture before treating it as settled.
