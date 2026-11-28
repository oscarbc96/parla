# GIF search: Giphy

A Parla plugin that adds Giphy as a GIF provider in the media picker. It
implements the `media-provider` extension point: keyword search, trending,
and resolving one GIF by id.

## What leaves the machine

Every search and trending request goes to `api.giphy.com` and carries:

- the search text (search only, cut to 50 characters),
- the page `limit` and `offset`,
- the content rating filter,
- your Giphy API key.

The picker then loads the returned GIFs and thumbnails from `media.giphy.com`,
`media0.giphy.com` to `media4.giphy.com`, and `i.giphy.com`. Those are image
requests made by Parla; the plugin only returns the URLs unchanged. Nothing is
sent anywhere else and the plugin has no telemetry.

The API key is stored by Parla in the operating system's keychain (the secret
store), not in the settings file, and never appears in the plugin's log or in
an error message.

## Getting a key

Create an API key in the Giphy developer dashboard at
<https://developers.giphy.com/dashboard/>, then paste it into the plugin's
**API key** setting.

## Requests

Taken from <https://developers.giphy.com/docs/api/endpoint> (Search,
Trending and Get GIF by ID endpoints) and
<https://developers.giphy.com/docs/api/schema> (the GIF object and its
`images` renditions).

```text
GET https://api.giphy.com/v1/gifs/search?api_key={key}&q={query}&limit={limit}&offset={offset}&rating={rating}
GET https://api.giphy.com/v1/gifs/trending?api_key={key}&limit={limit}&offset={offset}&rating={rating}
GET https://api.giphy.com/v1/gifs/{id}?api_key={key}
```

- `q` is percent-encoded and cut to 50 characters.
- `rating` is `g`, `pg`, `pg-13` or `r`; Parla's `unrated` limit maps to `r`.
- The cursor is the next `offset` as a decimal string. It is `None` when the
  page is empty, `offset + count >= total_count`, or the next offset would be
  past Giphy's cap (4,999 for search, 499 for trending).
- Search items use `fixed_width_small` (falling back to `fixed_width`) as the
  preview and `original` (falling back to `downsized`) as the full GIF. Width,
  height and byte size come from the full rendition; an item without a usable
  width or height is skipped.
- Resolving uses the by-id endpoint. A missing id is `not-found`.

## Attribution

Each result carries `Powered by GIPHY`, as Giphy's API terms require. No Giphy
logo or other Giphy asset is bundled with the plugin.

## Licence

`Apache-2.0 OR MIT`.
