# GIF search: Klipy

A Parla plugin that adds Klipy as a GIF provider in the media picker. It
implements the `media-provider` extension point: keyword search, trending,
and resolving one GIF by slug.

## What leaves the machine

Every search and trending request goes to `api.klipy.com` and carries:

- the search text (search only),
- the page number and page size,
- the content filter level,
- your Klipy app key (it is a path segment of the API URL).

The picker then loads the returned GIFs and thumbnails from
`static.klipy.com`, `static1.klipy.com` and `static2.klipy.com`. Those are
image requests made by Parla; the plugin only returns the URLs unchanged.
Nothing is sent anywhere else and the plugin has no telemetry.

The app key is stored by Parla in the operating system's keychain (the secret
store), not in the settings file, and never appears in the plugin's log or in
an error message.

### Privacy note: no `customer_id`

Klipy's API documents `customer_id` as optional, so this plugin does not send
it and does not generate or store any user identifier. The only identifiers
in a request are the app key and the search text.

## Getting a key

Create a Klipy app in the Partner Panel at <https://partner.klipy.com/>
(navigate to API Keys and create your platform), then paste the app key into
the plugin's **API key** setting.

## Requests

Taken from the Klipy API reference pages exported as `gifs-search-api`,
`gifs-trending-api` and `gifs-items-api` (they list the endpoint paths and the
`page`, `per_page`, `q`, `customer_id`, `content_filter`, `format_filter` and
`slugs` parameters).

```text
GET https://api.klipy.com/api/v1/{app_key}/gifs/search?page={page}&per_page={per_page}&q={query}&content_filter={filter}
GET https://api.klipy.com/api/v1/{app_key}/gifs/trending?page={page}&per_page={per_page}&content_filter={filter}
GET https://api.klipy.com/api/v1/{app_key}/gifs/items?slugs={slug}
```

- `q` and `slugs` are percent-encoded.
- `content_filter` is `high` for Parla's `g` limit, `medium` for `pg`, `low`
  for `pg13`, and `off` for `r` and `unrated` (stricter when in doubt).
- The cursor is the next page number. It is `None` when the response's
  `has_next` is false.
- Klipy requires at least 8 items per search page. The plugin requests 8 when
  the picker asks for fewer and returns only the requested number, so a
  search page smaller than 8 skips the rest of that API page. The picker
  normally asks for more than 8.
- Search previews use `sm` (falling back to `xs`); full GIFs use `md`
  (falling back to `hd`). Width, height and byte size come from the full
  rendition; an item without a usable width or height is skipped.
- The item id is Klipy's `slug`, because the items endpoint resolves by slug.
  A missing slug is `not-found`.

## Attribution

Each result carries `Powered by KLIPY`. Klipy's attribution guidance also asks
for a "Search KLIPY" placeholder in the search field; the Parla media picker
owns that field, so the plugin cannot set it.

## Licence

`Apache-2.0 OR MIT`.
