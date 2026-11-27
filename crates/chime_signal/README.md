# chime_signal

Provider-neutral signalling for Chime-style meeting control websockets (spec
section 6.6, layer 3): the wire messages, the frame codec and a sans-IO state
machine. No sockets, threads or timers.

## Frames

`encode` prefixes `0x05`; `decode` accepts a `0x02` or `0x05` first byte,
rejects empty, oversized (over 1 MiB), unknown-prefix, malformed and
unknown-type input with an `Error`, and never panics.

## State machine

`Machine::handle(now, Input) -> Vec<Output>` is the whole API, plus
`next_deadline()`; the caller owns the socket and the clock. Inputs: `Opened`,
`Frame(bytes)`, `Tick`, `LocalOffer { sdp }`, `SetMuted(bool)`, `Leave`,
`Closed`, `ClosedNormally` (a normal close code: a clean `Left` once joined).
Outputs: `Send(bytes)` to write to the socket, and `Event`s:
`TurnCredentials` (secrets in a zeroizing, redacted `Secret`), `RemoteAnswer`,
`Participants`, `Index`, `Error`, `Left`, `Failed`. `Tick` carries no payload:
every input takes the current `Instant` as the first argument of `handle`, so
a deadline is never judged against a stale time.

Sequence, as the Chime JS SDK does it (`DefaultSignalingClient`,
`SubscribeAndReceiveSubscribeAckTask`): `Opened` sends `JOIN` (protocol version
2, flag `HAS_STREAM_UPDATE`, random audio session id, client details,
`wants_compressed_sdp = false`); `JOIN_ACK` emits `TurnCredentials`; `INDEX`
emits `Index`; once the caller's `LocalOffer` is in (it may arrive earlier and
is held) the machine sends `SUBSCRIBE` (duplex `RX` like an audio-only Chime SDK client, `sdp_offer`, `audio_host`,
`audio_checkin = false`, one audio `send_streams` descriptor labelled with our attendee id, `audio_muted`); `SUBSCRIBE_ACK` emits `RemoteAnswer` and
the state is `Joined`. `AUDIO_STREAM_ID_INFO` emits `Participants` (the entries
the frame carries; the caller merges deltas). The join ack, the index and the
subscribe ack each have 10 s, else `Failed`. Every server `PING` is answered
with a `PONG` of the same id; a client `PING` goes out every 10 s from `Tick`
as a keepalive. A frame that does not parse is dropped, so a newer server
cannot end a call with a frame type this proto lacks.

Mute: `SetMuted` sends an `AUDIO_CONTROL` frame with `muted`, which is what the
SDK's `DefaultSignalingClient.mute` sends. Before the subscribe the value rides
in `audio_muted`; a change made while the subscribe ack is pending is sent
right after the ack.

Leave: `Leave` sends `LEAVE` and waits for `LEAVE_ACK` or the socket close,
then emits `Left` (after 2 s without either, the SDK's close timeout). A close
at any other time is `Failed`.

The caller sleeps until `next_deadline()` and feeds `Tick`; there are no
internal timers and nothing to poll. The prost message of a received join ack
holds the TURN credentials as plain strings until it is dropped inside
`handle`; it is never stored or logged.

## Vendored protocol

`proto/SignalingProtocol.proto` is copied **unmodified** from the public
Apache-2.0 repository `aws/amazon-chime-sdk-js`, path
`protocol/SignalingProtocol.proto`, at upstream commit
`eb93c7d902f8952edf1b2b2abb83cb23d8802ccf` (fetched 2026-11-21). `LICENSE` is
that repository's Apache-2.0 text; see `NOTICE`. Update by re-fetching the file
and changing the commit above; never edit the proto by hand.

`build.rs` compiles it with `protox` (a pure Rust protobuf compiler) and
`prost-build`, so no `protoc` is needed. Versions are pinned exactly in the
workspace `Cargo.toml`: `prost` 0.14.6, `prost-build` 0.14.6, `protox` 0.10.0.
