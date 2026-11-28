//! Klipy media provider for the Parla GIF picker.
//!
//! Search and trending follow `docs.klipy.com`:
//! `GET https://api.klipy.com/api/v1/{app_key}/gifs/search` and
//! `/gifs/trending` with `page`, `per_page`, `q` and `content_filter`; the
//! items endpoint resolves a saved GIF by slug. Media URLs are passed through
//! as the API returns them.

use parla_plugin_sdk::host::http::{self, Method, Request};
use parla_plugin_sdk::host::secrets;
use parla_plugin_sdk::host::{self, LogLevel};
use parla_plugin_sdk::media::{ContentRating, MediaItem, MediaPage};
use parla_plugin_sdk::{Error, Plugin, export};
use serde_json::Value;

const SERVICE: &str = "Klipy";
const API_HOST: &str = "https://api.klipy.com";
const SEARCH_PATH: &str = "/gifs/search";
const TRENDING_PATH: &str = "/gifs/trending";
const ITEMS_PATH: &str = "/gifs/items";
const ATTRIBUTION: &str = "Powered by KLIPY";

/// Klipy's search endpoint requires at least 8 items per page; trending
/// accepts 1. Both cap at 50.
const MIN_SEARCH_PAGE: u32 = 8;
const MAX_LIMIT: u32 = 50;
const TIMEOUT_MS: u32 = 5_000;

pub struct Klipy;

impl Plugin for Klipy {
    fn media_search(
        id: String,
        query: String,
        cursor: Option<String>,
        limit: u32,
        max_rating: ContentRating,
    ) -> Result<MediaPage, Error> {
        if id != "klipy" {
            return Err(Error::NotFound);
        }
        let key = api_key()?;
        let page = cursor_page(cursor.as_deref())?;
        let limit = limit.clamp(1, MAX_LIMIT);
        let per_page = limit.max(MIN_SEARCH_PAGE);
        let url = format!(
            "{API_HOST}/api/v1/{}/gifs/search?page={page}&per_page={per_page}&q={}&content_filter={}",
            encode(&key),
            encode(&query),
            filter_param(max_rating),
        );
        let value = get(&url, SEARCH_PATH, None)?;
        let mut page = page_from_envelope(&value, page, max_rating)?;
        page.items.truncate(limit as usize);
        Ok(page)
    }

    fn media_trending(
        id: String,
        cursor: Option<String>,
        limit: u32,
        max_rating: ContentRating,
    ) -> Result<MediaPage, Error> {
        if id != "klipy" {
            return Err(Error::NotFound);
        }
        let key = api_key()?;
        let page = cursor_page(cursor.as_deref())?;
        let limit = limit.clamp(1, MAX_LIMIT);
        let url = format!(
            "{API_HOST}/api/v1/{}/gifs/trending?page={page}&per_page={limit}&content_filter={}",
            encode(&key),
            filter_param(max_rating),
        );
        let value = get(&url, TRENDING_PATH, None)?;
        page_from_envelope(&value, page, max_rating)
    }

    fn media_resolve(id: String, item_id: String) -> Result<MediaItem, Error> {
        if id != "klipy" {
            return Err(Error::NotFound);
        }
        let key = api_key()?;
        let url = format!(
            "{API_HOST}/api/v1/{}/gifs/items?slugs={}",
            encode(&key),
            encode(&item_id),
        );
        let value = get(&url, ITEMS_PATH, Some(404))?;
        if value.get("result").and_then(Value::as_bool) == Some(false) {
            return Err(Error::NotFound);
        }
        value
            .get("data")
            .and_then(|data| data.get("data"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(map_item)
            .ok_or(Error::NotFound)
    }
}

export!(Klipy with [media_provider]);

/// The key is read on every call; a missing or blank key is the picker's
/// set-up state and never reaches the network.
fn api_key() -> Result<String, Error> {
    match secrets::secret_get("api_key")? {
        Some(key) if !key.trim().is_empty() => Ok(key),
        _ => Err(Error::Invalid("api key".into())),
    }
}

/// The cursor is the next page number; the first page is 1.
fn cursor_page(cursor: Option<&str>) -> Result<u64, Error> {
    match cursor.filter(|cursor| !cursor.is_empty()) {
        None => Ok(1),
        Some(text) => match text.parse() {
            Ok(page) if page > 0 => Ok(page),
            _ => Err(Error::Invalid("cursor".into())),
        },
    }
}

/// Fetches a JSON document. Logs only the path and status: the key is a path
/// segment and must never reach the log.
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
    requested_page: u64,
    max_rating: ContentRating,
) -> Result<MediaPage, Error> {
    if value.get("result").and_then(Value::as_bool) == Some(false) {
        return Err(Error::Failed(format!("{SERVICE} returned 200")));
    }
    let data = value
        .get("data")
        .ok_or_else(|| Error::Failed(format!("{SERVICE} returned 200")))?;
    let items = data
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Failed(format!("{SERVICE} returned 200")))?;
    let page = data
        .get("current_page")
        .and_then(number_u64)
        .unwrap_or(requested_page);
    let has_next = data
        .get("has_next")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let next = if has_next {
        page.checked_add(1).map(|page| page.to_string())
    } else {
        None
    };
    Ok(MediaPage {
        items: items
            .iter()
            .filter_map(map_item)
            .filter(|item| allows(item.rating, max_rating))
            .collect(),
        next,
    })
}

fn map_item(value: &Value) -> Option<MediaItem> {
    let id = text(value, "slug")?;
    let file = value.get("file")?;
    let (preview_url, _, _, _) = rendition(file, &["sm", "xs"])?;
    let (full_url, width, height, bytes) = rendition(file, &["md", "hd"])?;
    let title = text(value, "title")
        .or_else(|| text(value, "alt_text"))
        .unwrap_or_else(|| id.clone());
    Some(MediaItem {
        id,
        preview_url,
        full_url,
        mime_type: "image/gif".into(),
        width,
        height,
        bytes,
        title,
        // Klipy's documented response has no per-item rating; if one appears,
        // map the same strings Giphy uses, otherwise report it unrated.
        rating: text(value, "rating")
            .map(|rating| rating_from_klipy(&rating))
            .unwrap_or(ContentRating::Unrated),
        attribution: Some(ATTRIBUTION.into()),
    })
}

/// The first rendition in `levels` with a GIF URL and non-zero dimensions.
fn rendition(file: &Value, levels: &[&str]) -> Option<(String, u32, u32, u64)> {
    for level in levels {
        let Some(gif) = file.get(*level).and_then(|level| level.get("gif")) else {
            continue;
        };
        let Some(url) = text(gif, "url") else {
            continue;
        };
        let Some(width) = dimension(gif, "width") else {
            continue;
        };
        let Some(height) = dimension(gif, "height") else {
            continue;
        };
        let bytes = gif.get("size").and_then(number_u64).unwrap_or(0);
        return Some((url, width, height, bytes));
    }
    None
}

fn dimension(rendition: &Value, key: &str) -> Option<u32> {
    let number = rendition.get(key).and_then(number_u64)?;
    u32::try_from(number).ok().filter(|number| *number != 0)
}

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

fn rating_from_klipy(rating: &str) -> ContentRating {
    match rating {
        "g" => ContentRating::G,
        "pg" => ContentRating::Pg,
        "pg-13" | "pg13" => ContentRating::Pg13,
        "r" => ContentRating::R,
        _ => ContentRating::Unrated,
    }
}

/// Klipy normally reports no per-item rating, so an unrated item passes; a
/// rating it does give must not be above the user's limit.
fn allows(rating: ContentRating, max: ContentRating) -> bool {
    if rating == ContentRating::Unrated || max == ContentRating::Unrated {
        return true;
    }
    rank(rating) <= rank(max)
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

/// Klipy's `content_filter` is `off`, `low`, `medium` or `high`; stricter
/// when in doubt.
fn filter_param(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::G => "high",
        ContentRating::Pg => "medium",
        ContentRating::Pg13 => "low",
        ContentRating::R | ContentRating::Unrated => "off",
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
