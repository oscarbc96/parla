# huddles

Seen in the 2026-11-13 `huddle-typing` capture (join, update, leave of a huddle
in a test channel). Fixtures: `crates/providers/slack/realtime/tests/fixtures/spike/`
(ws) and `crates/providers/slack/api/tests/fixtures/spike/` (http). See also
[realtime.md](realtime.md).

### rooms.join

Kind: http. Params: `channel_id`, `regions`, `multidevice` (plus the usual
envelope). Response keys: `call{call_id,...}`, `canvas{canvas_file_id,root_thread_ts,thread_channel_id}`,
`huddle{id,channels,created_by,date_start,date_end,has_ended,participants,participant_history,pending_invitees,thread_root_ts,huddle_link,...}`,
`ok`. Request params seen: `token`, `channel_id`, `regions` (a media region such as `eu-west-3`), `multidevice` (`false`), `_x_reason`, `_x_mode`, `_x_app_name` (`client`), `_x_sonic`; Parla sends `_x_reason=rooms-join`, `_x_mode=online`, `_x_app_name=client`, `_x_sonic=true`. Verified 2026-11-21 against the test workspace (one call, shapes only; no value was kept). The real response carries media-service join credentials and meeting URLs, so it is never fixtured: the synthetic `crates/providers/slack/api/tests/fixtures/huddles/rooms-join-native.json` has the same key paths with fake values.

Key paths and types (2026-11-21), under `call`:
`call_id: string`, `survey_percent: number`,
`free_willy.attendee`: `AttendeeId: string`, `ExternalUserId: string`, `JoinToken: string` (secret), `Capabilities.{Audio,Content,Video}: string`, `EventActions: null`;
`free_willy.meeting`: `MeetingId: string`, `ExternalMeetingId: string`, `MeetingArn: string`, `MediaRegion: string`, `is_prewarmed: bool`, `MeetingFeatures: null`, `MeetingHostId: null`, `PrimaryMeetingId: null`, `TenantIds: array (empty)`,
`free_willy.meeting.MediaPlacement`: `SignalingUrl`, `AudioHostUrl`, `AudioFallbackUrl`, `TurnControlUrl`, `EventIngestionUrl`, `ScreenDataUrl`, `ScreenSharingUrl`, `ScreenViewingUrl` (all `string`).
Other top-level keys: `canvas{canvas_file_id,root_thread_ts,thread_channel_id: string}`, `ok: bool`, and `huddle{id,created_by,name,locale,background_id,external_unique_id,huddle_link,thread_root_ts: string; date_start,date_end: number; has_ended: bool; channels: array(string); participants,participant_history,attached_file_ids: array; prototypes{is_prewarmed,is_scheduled: bool}; recording{can_record_summary: string}}`.

Leaving: no leave call is known. A `rooms.join` without a media connection leaves the huddle running with no participants for at least several seconds; the web client leaves by closing its media connection (client `leave` frame, see "Media"). Parla maps `call.free_willy` to `domain::NativeCallJoin` (`slack_api::WebClient::rooms_join`); the mapping fails with an error, never a panic, when a required field (join token, attendee id, external user id, meeting id, signalling URL, audio host URL) is missing or empty, and `TurnControlUrl` is optional. Also seen: `huddles.get` (`huddle_id`),
`huddles.history` (`limit`), `screenhero.rooms.info` (`room`); the
`screenhero.rooms.proxyLogs` and `proxyMeetingEvents` calls are media-client
telemetry and are not fixtured.

### sh_room_join, sh_room_update, sh_room_leave (ws)

`{"type":..,"event_ts":..,"ts":..,"huddle":{"channel_id":..},"room":{...},"user":..}`
. `room` holds `id`, `channels`, `created_by`,
`date_start`, `date_end` (non-zero in the final update), `has_ended`,
`participants`, `participant_history`, `participants_camera_on/off`,
`participants_screenshare_on/off`, `pending_invitees`, `canvas_thread_ts`,
`call_family: "huddle"`, `media_backend_type`. Order seen: join, then
`huddle_invite_cancel`, leave, and a final update with `date_end` set.
Fixtures: `ws-sh_room_join.json`, `ws-sh_room_update.json`,
`ws-sh_room_leave.json`, `ws-huddle_invite_cancel.json`.

### user_huddle_changed (ws)

`user` object whose `profile.huddle_state` is `default_unset` or `in_a_huddle`
(with `huddle_state_call_id`) and `huddle_state_expiration_ts`. Arrives with
`user_invalidated` and `user_change` on every join and leave. Fixture:
`ws-user_huddle_changed.json`. The huddle also posts a `message` with subtype
`huddle_thread` (`ws-message-huddle_thread.json`).

Parla can show "in a huddle" from `user_huddle_changed` alone; `sh_room_*` adds
participants.

### Direct huddle URL

`huddle.huddle_link` in `rooms.join` and `huddles.get` (fixtured in
`api/tests/fixtures/spike/huddles.get.json`) and in the `sh_room_join` event is
`https://app.slack.com/huddle/<team id>/<channel id>`. Parla builds
that shape with `slack_auth::webview::huddle_url`. Whether
the URL opens the huddle surface for a signed-in session is still unverified.
Media hosts are in "Media" below.

### huddles/list and huddles/info (edge API)

Seen in the 2026-11-18 captures. Kind: http, on the edge host, not `/api/`:
`POST https://edgeapi.slack.com/cache/<team id>/huddles/list?_x_app_name=client&fp=..&_x_num_retries=0`.
Body: JSON sent as `content-type: text/plain;charset=UTF-8`, only
`{"token":"<xoxc>"}` (the session cookie rides along). The web client calls it
once on page load; it returns every running huddle in the team that the user can
see, so no channel ids are needed. `huddles/info` (same host, body
`{"channel_ids":[...],"token":..}`) has the same response shape and is what the
client calls afterwards to refresh single channels.
Response: `{"ok":true,"huddles":[{call_id,channel_id,created_by,start_date,active_members,dropped_members,pending_invitees,last_invite_status_by_user,knocks,expiration,is_private,is_scheduled,background_id,locale,name,recording{...},thread_root_ts,updated}]}`.
`huddles.history` lists ended rooms only, so it cannot give the active state.
Fixtures in `crates/providers/slack/api/tests/fixtures/huddles-active/`:
`0205-post.json` (`huddles/list`, a channel huddle), `0002-post.json`
(`huddles/info`, a ringing DM huddle), `0054-huddles.get.json` (`huddles.get`).

### ended_huddle_update (ws)

Seen in the 2026-11-18 capture. `{"type":"ended_huddle_update","huddle":{id,channels,created_by,date_start,date_end,has_ended:true,participant_history,participants,...}}`;
unlike `sh_room_*` the room is under `huddle`, not `room`. It follows the final
`sh_room_update` (`has_ended` true, `date_end` set) of a huddle. Fixtures for the
live set (DM join with a pending invitee, update, leave, channel final update,
`ended_huddle_update`): `crates/providers/slack/realtime/tests/fixtures/huddles-live/`.
Only frames from the `wss-primary` socket are kept; the media-service
signalling frames are never fixtured.

### Media (Chime SDK signalling)

Seen in the 2026-11-21 `huddle-media` capture (join a channel huddle in the
test workspace for about 20 s, then leave). Only shapes, hosts and frame
types were read; the raw capture was deleted, and no media or signalling
frame is fixtured (they carry meeting credentials, ICE credentials and SDP).

- `media_backend_type: "free_willy"` (in `sh_room_*`) is the Amazon Chime
  SDK. `rooms.join`'s `call.free_willy` holds the meeting and attendee
  credentials (`attendee.JoinToken` and the meeting's media placement).
- Hosts during the call: `signal.<region>.m.chime.aws` (websocket),
  `static.sdkassets.chime.aws` (the SDK's wasm and workers, not needed
  natively), `data.svc.<region>.ingest.chime.aws/v1/client-events`
  (telemetry, not needed), plus `screenhero.rooms.info`, `huddles.get`,
  `huddles.listBackgrounds` and `screenhero.rooms.proxyLogs` on the
  workspace host.
- Signalling websocket: `wss://signal.<region>.m.chime.aws/control/<meeting id>`
  with query params `X-Chime-Control-Protocol-Version`,
  `X-Amzn-Chime-Send-Close-On-Error`, `X-Amzn-Version`, `X-Amzn-User-Agent`.
  From the SDK source (`aws/amazon-chime-sdk-js` at
  `eb93c7d902f8952edf1b2b2abb83cb23d8802ccf`,
  `src/signalingclient/SignalingClientConnectionRequest.ts`, last changed in
  `0a2f560c408b71910cdb0bfbc77e7941bdef2b0d`; `src/task/OpenSignalingConnectionTask.ts`
  passes `urls.signalingURL` and `credentials.joinToken`): the URL is the
  meeting's `SignalingUrl` as is (the `/control/<meeting id>` path comes with
  it) plus `?X-Chime-Control-Protocol-Version=3&X-Amzn-Chime-Send-Close-On-Error=1`.
  The join token is presented as WebSocket subprotocols, so the upgrade
  request carries `Sec-WebSocket-Protocol: _aws_wt_session, <join token>`;
  there is no `Authorization` header and no token in the URL. The
  `X-Amzn-Version` and `X-Amzn-User-Agent` params seen in the capture are not
  in that file and are not required by it. A fresh upgrade capture (header
  names only) still confirms Slack's web client does the same.
- Meeting to join (`MeetingSessionConfiguration`, same SDK commit): the
  meeting response gives `Meeting.MeetingId`, `Meeting.MediaPlacement.SignalingUrl`,
  `AudioHostUrl` and `TurnControlUrl`; the attendee gives `Attendee.AttendeeId`,
  `ExternalUserId` and `JoinToken`. `domain::NativeCallJoin` maps them as
  `meeting_id`, `signaling_url`, `audio_host_url`, `turn_control_url`,
  `attendee_id`, `external_user_id` and `join_token` (secret). They sit in `rooms.join`'s
  `call.free_willy.attendee` and `.meeting` (see `rooms.join` above).
- Subscribe (SDK `SubscribeAndReceiveSubscribeAckTask` and
  `DefaultSignalingClient.subscribe`): a normal join sends `audio_checkin =
  false`, and when `audio_host` is set it declares one audio send stream in
  `send_streams`: media type audio, track label `AmazonChimeExpressAudio`,
  the attendee's own attendee id, stream id 1, group id 1, framerate 15, max
  bitrate 600 kbps, average bitrate 400000 bps. Duplex stays `RX` for audio
  only. With the stream declared (and `audio_checkin` false) the server sent
  downlink RTP to a native attendee; with `audio_checkin = true` and no send
  stream it sent none.
- Frames are binary: one type byte (`0x05` client to server, `0x02` server
  to client), then an `SdkSignalFrame` from Chime's published
  `SignalingProtocol.proto` (`aws/amazon-chime-sdk-js`, Apache-2.0). The
  sequence seen, by `SdkSignalFrame` field: `ping_pong` (20) both ways
  first, then client `join` (4), server
  `joinack` (5, TURN credentials), server `index` (8), client `sub` (6, SDP
  offer, about 1.8 KB), server `suback` (7, SDP answer, about 1.3 KB),
  server `audio_stream_id_info` (19) and `audio_metadata` (18, frequent),
  client `data_message` (23) and `client_metric` (22) periodically, server
  `bitrates` (14), client `leave` (11). 45 frames received and 10 sent in
  about 20 s.
- Speaking and sharing (from the published proto and the SDK source, not yet
  checked on a live huddle): `audio_metadata.attendee_states` carry
  `audio_stream_id`, `volume` (attenuation, 0 loudest, 127 silent) and
  `muted`; Parla counts a stream as speaking at `volume <= 39` unless muted,
  and names it through `audio_stream_id_info`. A screen share joins as a
  second attendee whose id ends in `#content`. `ExternalUserId` is taken as
  the Slack user id (the synthetic fixture uses `U...`).

### Parla's huddle client
`capabilities().huddles` and `capabilities().call_surface` are on.
- **Active state at load:** one `huddles/list` call (`WebClient::huddles_list` in
`slack_api`, `edge_base` in `ClientConfig`). `call_id` is the room id,
`active_members` the participants, `start_date` the start; a `D...` channel id is
a DM. Parla never sweeps channels with `huddles/info`. After that, `sh_room_join`,
`sh_room_update`, `sh_room_leave` and `ended_huddle_update` (an end) keep the
state live. `huddles.get` takes `huddle_id` (seen).
- **Decline (unverified):** `huddles.decline` with `huddle_id` and `channel_id`. The method
  name and params are a guess; the real call is unknown. It is a write: only
  a 429 is retried.
- **Ring:** derived, not a frame (the DM join with a pending invitee is seen in the 2026-11-18 capture). A `sh_room_join` whose `pending_invitees`
  (object keys, or a list) contains the signed-in user rings that user. No dedicated ring
  event is known. `huddle_invite_cancel` (seen) stops the ring; the callee-answered-elsewhere
  and unanswered cases are not captured. The 45 s ring timeout is a fallback value.
- **Tolerance:** every room field may be missing or `null`; rooms and frames without an id are
  dropped (frames stay `Unknown`).
- **Ring rules (Parla side):** rings only start from a `sh_room_join`, never from an update or
  a leave, and only while the huddle capability is on. A huddle the user declined or that
  timed out does not ring again. `user_huddle_changed` (`huddle_state`, seen) is advisory: it
  only updates the displayed in-huddle flag and never starts, ends or rings a huddle.

## Timeline cards (`slack_markup`)

- **Huddle card:** a `huddle_thread` message carries the huddle `room` (seen,
  `spikes` fixture `ws-message-huddle_thread.json`). `slack_markup::huddle_card` shows who
  (`participant_history`, else `participants`), start (`date_start`), end (`date_end`, 0 =
  running) and the `huddle_link` as the join URL; the join URL is dropped once `has_ended`.
- **Third-party call block (unverified):** a `call` block with `call.v1`. Provider name from
  `provider_name`, `app_name` or `external_display_id`, else "Call"; `join_url` (or
  `desktop_app_join_url`), `date_start`, `date_end`, `all_participants[].slack_id`. Never
  captured: the fixture `call_block_message.json` is synthetic and the field names are
  unverified. A missing URL or name still renders a card.
