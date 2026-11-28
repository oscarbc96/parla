//! Giphy media provider for the Parla GIF picker.
//!
//! Search and trending follow `developers.giphy.com/docs/api/endpoint`; the
//! response envelope and the `images` renditions follow
//! `developers.giphy.com/docs/api/schema`. Only the API host is called; media
//! URLs are passed through as the API returns them.

use parla_plugin_sdk::host::http::{self, Method, Request};
use parla_plugin_sdk::host::secrets;
use parla_plugin_sdk::host::{self, LogLevel};
use parla_plugin_sdk::media::{ContentRating, MediaItem, MediaPage};
use parla_plugin_sdk::{Error, Plugin, export};
use serde_json::Value;

const SERVICE: &str = "Giphy";
const API_HOST: &str = "https://api.giphy.com";
const SEARCH_PATH: &str = "/v1/gifs/search";
const TRENDING_PATH: &str = "/v1/gifs/trending";
const RESOLVE_PATH: &str = "/v1/gifs";
const ATTRIBUTION: &str = "Powered by GIPHY";

/// Giphy caps the search offset at 4,999 and the trending offset at 499.
const MAX_SEARCH_OFFSET: u64 = 4_999;
const MAX_TRENDING_OFFSET: u64 = 499;
/// Giphy accepts at most 50 characters in `q`.
const MAX_QUERY_CHARS: usize = 50;
const MAX_LIMIT: u32 = 50;
const TIMEOUT_MS: u32 = 5_000;

pub struct Giphy;

impl Plugin for Giphy {
    fn media_search(
        id: String,
        query: String,
        cursor: Option<String>,
        limit: u32,
        max_rating: ContentRating,
    ) -> Result<MediaPage, Error> {
        if id != "giphy" {
            return Err(Error::NotFound);
        }
        let key = api_key()?;
        let offset = cursor_offset(cursor.as_deref())?;
        let limit = limit.clamp(1, MAX_LIMIT);
        let query: String = query.chars().take(MAX_QUERY_CHARS).collect();
        let url = format!(
            "{API_HOST}{SEARCH_PATH}?api_key={}&q={}&limit={limit}&offset={offset}&rating={}",
            encode(&key),
            encode(&query),
            rating_param(max_rating),
        );
        let value = get(&url, SEARCH_PATH, None)?;
        page_from_envelope(&value, offset, limit, max_rating, MAX_SEARCH_OFFSET)
    }

    fn media_trending(
        id: String,
        cursor: Option<String>,
        limit: u32,
        max_rating: ContentRating,
    ) -> Result<MediaPage, Error> {
        if id != "giphy" {
            return Err(Error::NotFound);
        }
        let key = api_key()?;
        let offset = cursor_offset(cursor.as_deref())?;
        let limit = limit.clamp(1, MAX_LIMIT);
        let url = format!(
            "{API_HOST}{TRENDING_PATH}?api_key={}&limit={limit}&offset={offset}&rating={}",
            encode(&key),
            rating_param(max_rating),
        );
        let value = get(&url, TRENDING_PATH, None)?;
        page_from_envelope(&value, offset, limit, max_rating, MAX_TRENDING_OFFSET)
    }

    fn media_resolve(id: String, item_id: String) -> Result<MediaItem, Error> {
        if id != "giphy" {
            return Err(Error::NotFound);
        }
        let key = api_key()?;
        let url = format!(
            "{API_HOST}{RESOLVE_PATH}/{}?api_key={}",
            encode(&item_id),
            encode(&key),
        );
        let value = get(&url, "/v1/gifs/<id>", Some(404))?;
        match value.get("data") {
            Some(data) if data.is_object() => map_item(data).ok_or(Error::NotFound),
            Some(data) => data
                .as_array()
                .and_then(|items| items.first())
                .and_then(map_item)
                .ok_or(Error::NotFound),
            None => Err(Error::NotFound),
        }
    }
}

export!(Giphy with [media_provider]);

/// The key is read on every call; a missing or blank key is the picker's
/// set-up state and never reaches the network.
fn api_key() -> Result<String, Error> {
    match secrets::secret_get("api_key")? {
        Some(key) if !key.trim().is_empty() => Ok(key),
        _ => Err(Error::Invalid("api key".into())),
    }
}

/// The cursor is the next offset as a decimal string.
fn cursor_offset(cursor: Option<&str>) -> Result<u64, Error> {
    match cursor.filter(|cursor| !cursor.is_empty()) {
        None => Ok(0),
        Some(text) => text.parse().map_err(|_| Error::Invalid("cursor".into())),
    }
}

/// Fetches a JSON document. Logs only the path and status: the API key is in
/// the query string and must never reach the log.
fn get(url: &str, path: &str, not_found: Option<u16>) -> Result<Value, Error> {
    let request = Request {
        method: Method::Get,
        url: url.into(),
        headers: vec![("accept".into(), "application/json".into())],
        body: None,
        timeout_ms: TIMEOUT_MS,
    };
    let response = http::fetch(&request)?;
    let status = response.status;
    let _ = host::log(LogLevel::Debug, &format!("GET {path} {status}"));
    if Some(status) == not_found {
        return Err(Error::NotFound);
    }
    match status {
        401 | 403 => return Err(Error::Invalid("api key".into())),
        429 => return Err(Error::Limit("rate".into())),
        200..=299 => {}
        _ => return Err(Error::Failed(format!("{SERVICE} returned {status}"))),
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| Error::Failed(format!("{SERVICE} returned {status}")))
}

fn page_from_envelope(
    value: &Value,
    offset: u64,
    limit: u32,
    max_rating: ContentRating,
    max_offset: u64,
) -> Result<MediaPage, Error> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Failed(format!("{SERVICE} returned 200")))?;
    let pagination = value.get("pagination");
    let count = pagination
        .and_then(|p| p.get("count"))
        .and_then(number_u64)
        .unwrap_or(data.len() as u64);
    let current = pagination
        .and_then(|p| p.get("offset"))
        .and_then(number_u64)
        .unwrap_or(offset);
    let total = pagination
        .and_then(|p| p.get("total_count"))
        .and_then(number_u64);
    let next = next_cursor(current, count, total, limit, max_offset);
    let items = data
        .iter()
        .filter_map(map_item)
        .filter(|item| allows(item.rating, max_rating))
        .collect();
    Ok(MediaPage { items, next })
}

/// The next `offset`, or `None` when the result set ends, the page is empty,
/// or the service's offset cap is reached.
fn next_cursor(
    offset: u64,
    count: u64,
    total: Option<u64>,
    limit: u32,
    max_offset: u64,
) -> Option<String> {
    if count == 0 {
        return None;
    }
    let next = offset.checked_add(count)?;
    if next > max_offset {
        return None;
    }
    match total {
        Some(total) if next >= total => None,
        // `total_count` is absent on some endpoints: a short page is the end.
        None if count < u64::from(limit) => None,
        Some(_) | None => Some(next.to_string()),
    }
}

fn map_item(value: &Value) -> Option<MediaItem> {
    let id = text(value, "id")?;
    let images = value.get("images")?;
    let preview = images
        .get("fixed_width_small")
        .and_then(rendition_url)
        .or_else(|| images.get("fixed_width").and_then(rendition_url))?;
    let full = images
        .get("original")
        .filter(|rendition| rendition_url(rendition).is_some())
        .or_else(|| images.get("downsized"))?;
    let full_url = rendition_url(full)?;
    let width = dimension(full, "width")?;
    let height = dimension(full, "height")?;
    let bytes = full.get("size").and_then(number_u64).unwrap_or(0);
    let title = text(value, "title")
        .or_else(|| text(value, "alt_text"))
        .or_else(|| text(value, "slug"))
        .unwrap_or_default();
    let rating = value
        .get("rating")
        .and_then(Value::as_str)
        .map(rating_from_giphy)
        .unwrap_or(ContentRating::Unrated);
    Some(MediaItem {
        id,
        preview_url: preview,
        full_url,
        mime_type: "image/gif".into(),
        width,
        height,
        bytes,
        title,
        rating,
        attribution: Some(ATTRIBUTION.into()),
    })
}

fn rendition_url(rendition: &Value) -> Option<String> {
    text(rendition, "url")
}

fn dimension(rendition: &Value, key: &str) -> Option<u32> {
    let number = rendition.get(key).and_then(number_u64)?;
    u32::try_from(number).ok().filter(|number| *number != 0)
}

/// Giphy sends `width`, `height` and `size` as decimal strings, but accept
/// numbers too so a schema change does not drop every item.
fn number_u64(value: &Value) -> Option<u64> {
    match value {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_u64(),
        _ => None,
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn rating_param(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::G => "g",
        ContentRating::Pg => "pg",
        ContentRating::Pg13 => "pg-13",
        ContentRating::R | ContentRating::Unrated => "r",
    }
}

fn rating_from_giphy(rating: &str) -> ContentRating {
    match rating {
        "g" => ContentRating::G,
        "pg" => ContentRating::Pg,
        "pg-13" => ContentRating::Pg13,
        "r" => ContentRating::R,
        _ => ContentRating::Unrated,
    }
}

/// The service filters, and this keeps the WIT promise if it ever returns a
/// rating above the user's limit. An unrated item is only shown when the
/// user's limit is `unrated`.
fn allows(rating: ContentRating, max: ContentRating) -> bool {
    match (rating, max) {
        (_, ContentRating::Unrated) => true,
        (ContentRating::Unrated, _) => false,
        _ => rank(rating) <= rank(max),
    }
}

fn rank(rating: ContentRating) -> u8 {
    match rating {
        ContentRating::G => 0,
        ContentRating::Pg => 1,
        ContentRating::Pg13 => 2,
        ContentRating::R => 3,
        ContentRating::Unrated => 4,
    }
}

/// Percent-encodes every byte outside RFC 3986's unreserved set (uppercase
/// hex), so a query with spaces, `&`, `/` or non-ASCII text is one value.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                out.push(char::from(HEX[usize::from(byte >> 4)]));
                out.push(char::from(HEX[usize::from(byte & 0x0f)]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
