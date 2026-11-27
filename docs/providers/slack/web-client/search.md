# search

## Message, file, people and channel search (UNVERIFIED)

Implemented in `slack_api` (`search.rs`) and `slack` (`search.rs`) from the
plan and synthetic fixtures (`crates/providers/slack/api/tests/fixtures/search/`);
no capture of the web client was made. Treat every detail below as unverified
until someone captures it with the `web-client-recon` skill and the fixtures are replaced.

- Calls: the `search.modules.<module>` family, `module` in `messages`,
  `files`, `people`, `channels`. Form fields: `module`, `query`, `count`,
  and `cursor` (omitted on the first page). Messages also send
  `highlight=true`, `sort=timestamp`, `sort_dir=desc`. Tier assumed T2.
- Messages response (shape seen once on a real account, then redacted into
  `messages_paged.json`): `ok`, `pagination: {total_count, page, per_page,
  page_count, first, last}`, `query`, `module`, `filters`, `items: [{iid, team,
  channel: {id, name, is_channel, ...}, messages: [{ts, user, iid, text,
  permalink, username, blocks, extracts, ...}]}]` (one message per item), and
  `filter_suggestions`. There is no cursor: paging is by number, the next
  page is `page + 1` while `page < page_count`, and a follow-up request sends
  it as the `page` form field (the `page` field itself is still unverified).
  A `pagination.next_cursor`, when present, still wins and goes out as
  `cursor`. With
  `highlight` the match in `text` sits between U+E000 and U+E001; Parla strips
  them and flattens markup (`slack_markup::plain_text`) for the snippet.
- `count` is clamped to 1..=100 (assumed server maximum). Hits with an empty channel id or ts are dropped.
- Files response: `items: [{file, channel?: {id}, user?, ts?}]` plus the same
  pagination; without `channel` the first `shares` channel is used.
- People and channels responses: `items: [<user>]` / `items: [<conversation>]`,
  one page, no cursor.
- Query text: free text with `"` balanced (an unpaired last quote is dropped) and every word containing `:` quoted so it cannot act as an operator,
  `in:#name` or `in:<#C..>`, `from:@name` or `from:<@U..>` (the id form when
  the name has spaces or quotes, or is unknown; the provider currently always
  sends the id form), `before:YYYY-MM-DD`, `after:YYYY-MM-DD`, and
  `has:file|link|reaction|pin`.
- Dates: the server reads `before:`/`after:` as days in the user's time zone;
  the local store reads them as whole UTC days. The two can differ by up to a
  day at the edges; not reconciled.
