# conversations

No sections yet.

## Fallback when `client.userBoot` lists no conversations

If `client.userBoot` returns neither `channels` nor `ims`, bootstrap lists the
user's conversations with `users.conversations`
(`types=public_channel,private_channel,mpim,im`, `exclude_archived=true`,
paged by cursor) and maps them like boot conversations, so the sidebar is
never left empty by a thin boot payload.

## Membership and group names

`client.userBoot` lists only the user's own conversations and leaves
`is_member` out of its `channels`, so bootstrap treats every channel it carries
as a membership (the `users.conversations` fallback does the same). A group DM
has no title: its name is `mpdm-<user>--<user>--...-<n>` made of user names
(handles), which the mapper splits into the group's participants. The app shows
the participants' display names once their members are loaded.

## Topic and member count

The conversation header shows `topic.value` and `num_members` when the
conversation objects carry them (`users.conversations`, `conversations.info`).
Boot payloads that leave them out show a header with the name only; they are
not stored in the local cache and fill in from the next listing.

## Opening a direct message (`conversations.open`)

The profile panel's "Message" button opens the one-to-one conversation with a
person we have none with: `conversations.open` with form fields `users=<id>` and
`return_im=true`; the answer carries `channel` (`id`, `is_im`, `user`), which is
mapped like any direct message and shown in the sidebar. Opening an existing
direct message returns the same channel (`already_open`). Unverified against a
capture (public Web API method; the fixture
`crates/providers/slack/api/tests/fixtures/conversations/open.json` is
synthetic), so the web client's own call may differ.

## Open and closed direct messages

Direct and group conversations carry `is_open` (the official client lists
only open ones; a closed one reappears when a new message arrives). The mapper
reads `is_open` from `client.userBoot` `ims` and `channels` entries (group DMs)
and `users.conversations` results and maps it to the neutral
`Conversation.is_open`; the sidebar hides closed ones (search and jump still
reach them). When the field is absent the conversation counts as open, except a group DM
(`is_mpim`): the boot payload never carries `is_open` for those, so the mapper
infers it (`group_dm_fallback_open`). A group DM is open when `client.counts`
lists it with unreads or a `latest` within the last 30 days; one missing from
`client.counts` (which lists only conversations with recent state) or older is
closed. Closed ones are hidden like closed 1:1 DMs and reopen on a new message.
A group DM that carries `is_open` (from `users.conversations`) uses it as is.
Not verified against a capture: no `client.userBoot` response is saved yet (see
[boot.md](boot.md)), so which entries actually carry `is_open` is assumed from
the public conversation object. A live `message` in a closed conversation
reopens it in the app.

## Peer names

A DM is named after its peer: display name, else real name, else handle, else
the localized "Unknown" placeholder. The raw user id is never shown; the peer
is looked up on demand when the directory does not carry it.

## Channel admin

Seen in the 2026-11-24 web-client capture in the test workspace (shapes only;
fixtures are hand-written from them in
`crates/providers/slack/api/tests/fixtures/channel_admin/`). Every call is a
form POST with the session token; the web client also sends `_x_*` tracing
fields, which Parla leaves out. All answered `ok: true`.

| Method | Params the web client sends | Answer | Parla |
|---|---|---|---|
| `conversations.validateName` | `name`, `team_id` | `{ok}`; an error for a taken or invalid name | `validate_channel_name` (not called yet: `create` validates too) |
| `conversations.create` | `name`, `validate_name=true`, `team_id`; `is_private=true` for a private channel (verified in a live capture) | `{ok, channel}`: `id`, `created`, `creator`, `name`, `name_normalized`, `is_channel`, `is_group`, `is_mpim`, `is_private`, `is_org_shared`, `context_team_id`, `updated`, ... | `create_channel`; Parla sends no `team_id` (single-team sessions) |
| `conversations.join` | `channel`, `request_in_background=true` | `{ok, channel, ...}` | `join_channel` |
| `conversations.leave` | `channel` | `{ok}` | `leave_channel` |
| `conversations.archive` | `channel` | `{ok}` | `archive_channel` |
| `conversations.rename` | `channel`, `name`, `validate` | `{ok, channel}` | `rename_channel` (`validate=true`) |
| `conversations.setTopic` | `channel`, `topic` | `{ok, channel}` | `set_channel_topic` |
| `conversations.setPurpose` | `channel`, `purpose`, `name_tagging` | `{ok, channel}`; the UI calls the purpose "description" | `set_channel_purpose` (`name_tagging=true`) |
| `conversations.invite` | not captured (it would notify the second user); public Web API `channel`, `users` (comma-separated) | assumed `{ok, channel}`; a batch with someone already in fails as a whole with `already_in_channel` | `invite_to_channel`; on a per-person error with more than one person the provider asks again one by one, so the app says who could not be added |
| `conversations.members` | not captured; public Web API `channel`, `limit`, `cursor` | `{ok, members: [user id], response_metadata.next_cursor}` | `channel_members` (every page, `limit=1000`); the invite picker leaves these people out |
| `stars.add` / `stars.remove` | `channel` | `{ok}`; `already_starred` / `not_starred` when it already is (or is not) starred, which Parla treats as done | `star_channel`; the web client shows stars as the Favorites sidebar section |
| `search.modules.channels` (browse) | `module=channels`, `query`, `page=1`, `count=50`, `sort=recommended`, `channel_type=all`, `exclude_my_channels=0`, `search_only_my_channels=false`, `search_context=desktop_channel_browser` (live capture, empty query) | `{ok, items, pagination, filters, ...}`; each item `id`, `name`, `is_archived`, `member_count`, `is_member`, `context_team_id`, `is_private`, `purpose {value, creator, last_set}`, `timestamp` | `browse_channels` |

The browser's `pagination` is `{total_count, page, per_page, page_count,
first, last}` (live capture), so the next page is `page + 1` until
`page_count`. `member_count` maps to the neutral `member_count`,
`purpose.value` to `Conversation.purpose`. Parla asks once the typed text
settles (250 ms after the last key) and drops the page on its way when the
text changes again.

Stars are one-way for now: Parla sends `stars.add` and `stars.remove` but
does not read the stars back at start (no cheap list call captured), so a
star set in another client does not show in Parla.

Errors the provider maps to its own text (`crates/providers/slack/provider/src/channel_admin.rs`):
`name_taken`; `invalid_name*` and `no_channel` (invalid name);
`cant_leave_general`, `cant_archive_general`; `already_archived`,
`is_archived`; `method_not_supported_for_channel_type`, `not_authorized`,
`restricted_action` (not allowed); `already_in_channel`, `not_in_channel`,
`cant_invite_self`, `user_is_restricted` (and `user_is_ultra_restricted`,
`ura_max_channels`); `too_long` from `setTopic` and `setPurpose` (topic or
description too long). Others read as the generic provider error.

Channel names follow the provider's rule (`check_channel_name`): trimmed,
lowercase, spaces and dots become hyphens, at most 80 characters of letters,
digits, `-` and `_`. The create and edit dialogs ask the provider; the core
has no name rules of its own.

Live events (public event shapes, synthetic fixtures in
`crates/providers/slack/realtime/tests/fixtures/events/`, not captured):
`channel_rename` and `group_rename` carry the channel object with its new
`name`; `channel_archive`, `group_archive`, `channel_unarchive`,
`group_unarchive` carry the channel id; `channel_left` and `group_left` say
the signed-in user left. `member_joined_channel` and `member_left_channel`
for the signed-in user change its membership; for anyone else they change the
conversation's member list (when one is held) and its member count. Each maps
to a neutral `ConversationMetaChanged` or `ConversationMemberChanged`.

A live change of a conversation the app does not hold yet (the socket URL
carries `lazy_channels=1`) is not dropped: the core asks for it with
`conversations.info` (`channel=<id>`, `Priority::Visible`; the answer is a
`{channel}` object mapped like any conversation) and adds it with what the
event said about membership and archive state, so a channel created or joined
from another client appears without a reload. At most 32 ids are asked for at
once; a failed request is forgotten and the next change asks again. A fetched
conversation that is already held only refreshes its name, topic, description,
member count and archive state, never its read, mute or unread state.
