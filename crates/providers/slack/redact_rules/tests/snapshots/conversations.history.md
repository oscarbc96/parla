### conversations.history

Kind: http

Request params: `_x_id`, `channel`, `cursor`, `limit`

| Field | Type | Optional | Values seen |
|---|---|---|---|
| `has_more` | bool | no |  |
| `messages` | array<object> | no |  |
| `messages[].reply_count` | number | yes |  |
| `messages[].subtype` | string | yes | `channel_join` |
| `messages[].text` | string | no |  |
| `messages[].thread_ts` | string | yes |  |
| `messages[].ts` | string | no |  |
| `messages[].type` | string | no | `message` |
| `messages[].user` | string | no |  |
| `ok` | bool | no |  |
| `response_metadata` | object | yes |  |
| `response_metadata.next_cursor` | string | no |  |
