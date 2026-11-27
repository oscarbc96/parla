### items.list

Kind: http

Request params: `_x_id`, `cursor`, `folder`, `limit`

| Field | Type | Optional | Values seen |
|---|---|---|---|
| `items` | array<object> | no |  |
| `items[].child_count` | number | yes |  |
| `items[].parent_ts` | string | yes |  |
| `items[].subtype` | string | yes | `joined` |
| `items[].text` | string | no |  |
| `items[].ts` | string | no |  |
| `items[].type` | string | no | `item_added` |
| `items[].user` | string | no |  |
| `more` | bool | no |  |
| `ok` | bool | no |  |
| `response_meta` | object | yes |  |
| `response_meta.next_cursor` | string | no |  |
