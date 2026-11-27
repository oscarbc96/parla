# files

## Clips and uploads (provisional)

No capture yet; this is read from public sources and is provisional.
Clips carry `subtype` `slack_video` or `slack_audio` (not reliable
`mimetype`/`filetype`); video renditions are `mp4_low`, `mp4`, `hls`, with
`thumb_video`, `duration_ms`, `vtt`. The web client's own upload calls are
unconfirmed.
Parla maps `vtt` to the clip's captions (`VideoInfo.vtt`) and fetches it
with the host-checked `fetch_private`, capped at 1 MB while streaming.

## Files: info, upload, download (UNVERIFIED)

Implemented in `slack_api` from the public docs and synthetic fixtures
(`crates/providers/slack/api/tests/fixtures/files/`); no capture of the web
client was made. Treat every detail below as unverified until captured.

- `files.info` (`file`): response `{file: {...}}`. Thumbnails are
  `thumb_<N>` with `_w`/`_h`; pdf `thumb_pdf*`; video `thumb_video*`, `mp4`,
  `mp4_low`, `hls`, `duration_ms`. `mode: "tombstone"` marks a deleted file.
  `shares.{public,private}.<channel>[].ts` gives the sharing message.
- Upload: `files.getUploadURLExternal` (`filename`, `length`) returns
  `upload_url` and `file_id`; the bytes are POSTed raw to `upload_url`
  without credentials; `files.completeUploadExternal` (`files` JSON
  `[{id,title}]`, `channel_id`, `thread_ts`, `initial_comment`) shares them in
  one message. Unknown: the web client's own body encoding and extra params.
- Snippets: a staged code snippet is an ordinary text file uploaded with
  `snippet_type` (the language id, e.g. `rust`) added to
  `files.getUploadURLExternal`, which the public docs list as the syntax type of
  the snippet (UNVERIFIED against the web client). The provider-neutral
  `Upload.filetype` carries it; the Slack provider passes it through unchanged.
  A snippet's `files.info` has `filetype`, `lines` and `preview` (the first
  lines). Parla maps `lines` (or, without it, the preview's lines plus
  `lines_more`) to the neutral `FileRef.lines` so the card can say when its
  preview is only part of the file; the language comes from the file name.
- Download: only `https` on the hosts accepted by `is_file_host`
  (`crates/providers/slack/api/src/files.rs`); cookie plus
  `Authorization: Bearer` is sent; at most 3 redirects, each re-checked,
  credentials dropped on a hop to another host. Whether the bearer header is
  needed is unconfirmed.
- Clips: `subtype` `slack_video` / `slack_audio` decides, never `mimetype`.
