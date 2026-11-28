//! Media provider test guest.
//!
//! `media-search` fetches `https://api.example.test/search?q=<query>&rating=<max>`
//! with the `api_key` secret in `x-key` and returns the line-based items the
//! test scripts in the response body, one per line:
//!
//! ```text
//! id|preview-url|full-url|mime|width|height|bytes|title|rating|attribution
//! ```
//!
//! Queries the host tests use: `sleep` spins (deadline), `redirect` fetches a
//! URL the test redirects, `http` fetches a plain-http URL, `header` adds a
//! forbidden `Cookie` header, and `setting:<key>` reads one setting.

use parla_plugin_sdk::host::http::{self, Method, Request};
use parla_plugin_sdk::host::{secrets, settings};
use parla_plugin_sdk::media::{ContentRating, MediaItem, MediaPage};
use parla_plugin_sdk::types::Error;
use parla_plugin_sdk::{Plugin, export};

struct Media;

impl Plugin for Media {
    fn media_search(
        _id: String,
        query: String,
        _cursor: Option<String>,
        limit: u32,
        max_rating: ContentRating,
    ) -> Result<MediaPage, Error> {
        if query == "sleep" {
            let mut x: u64 = 1;
            loop {
                x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
            }
        }
        if let Some(key) = query.strip_prefix("setting:") {
            return match settings::setting(key)? {
                Some(_) => Ok(MediaPage {
                    items: Vec::new(),
                    next: None,
                }),
                None => Err(Error::Invalid("no setting".into())),
            };
        }
        let key =
            secrets::secret_get("api_key")?.ok_or_else(|| Error::Invalid("api key".into()))?;
        let url = match query.as_str() {
            "redirect" => "https://api.example.test/redirect".to_owned(),
            "http" => "http://api.example.test/search".to_owned(),
            _ => format!(
                "https://api.example.test/search?q={query}&rating={}",
                rating_name(max_rating)
            ),
        };
        let mut headers = vec![("x-key".to_owned(), key)];
        if query == "header" {
            headers.push(("Cookie".to_owned(), "x=1".to_owned()));
        }
        let response = http::fetch(&Request {
            method: Method::Get,
            url,
            headers,
            body: None,
            timeout_ms: 5_000,
        })?;
        Ok(page(&response.body, limit))
    }

    fn media_trending(
        _id: String,
        _cursor: Option<String>,
        limit: u32,
        max_rating: ContentRating,
    ) -> Result<MediaPage, Error> {
        let key =
            secrets::secret_get("api_key")?.ok_or_else(|| Error::Invalid("api key".into()))?;
        let response = http::fetch(&Request {
            method: Method::Get,
            url: format!(
                "https://api.example.test/trending?rating={}",
                rating_name(max_rating)
            ),
            headers: vec![("x-key".to_owned(), key)],
            body: None,
            timeout_ms: 5_000,
        })?;
        Ok(page(&response.body, limit))
    }

    fn media_resolve(_id: String, item_id: String) -> Result<MediaItem, Error> {
        let key =
            secrets::secret_get("api_key")?.ok_or_else(|| Error::Invalid("api key".into()))?;
        let response = http::fetch(&Request {
            method: Method::Get,
            url: format!("https://api.example.test/resolve?id={item_id}"),
            headers: vec![("x-key".to_owned(), key)],
            body: None,
            timeout_ms: 5_000,
        })?;
        line(&String::from_utf8_lossy(&response.body)).ok_or(Error::NotFound)
    }
}

export!(Media with [media_provider]);

fn rating_name(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::G => "g",
        ContentRating::Pg => "pg",
        ContentRating::Pg13 => "pg13",
        ContentRating::R => "r",
        ContentRating::Unrated => "unrated",
    }
}

fn page(body: &[u8], limit: u32) -> MediaPage {
    let text = String::from_utf8_lossy(body);
    MediaPage {
        items: text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(limit as usize)
            .filter_map(line)
            .collect(),
        next: None,
    }
}

fn line(text: &str) -> Option<MediaItem> {
    let f: Vec<&str> = text.split('|').collect();
    let [
        id,
        preview,
        full,
        mime,
        width,
        height,
        bytes,
        title,
        rating,
        attribution,
    ] = f.as_slice()
    else {
        return None;
    };
    Some(MediaItem {
        id: (*id).to_owned(),
        preview_url: (*preview).to_owned(),
        full_url: (*full).to_owned(),
        mime_type: (*mime).to_owned(),
        width: width.parse().ok()?,
        height: height.parse().ok()?,
        bytes: bytes.parse().ok()?,
        title: (*title).to_owned(),
        rating: match *rating {
            "g" => ContentRating::G,
            "pg" => ContentRating::Pg,
            "pg13" => ContentRating::Pg13,
            "r" => ContentRating::R,
            _ => ContentRating::Unrated,
        },
        attribution: (!attribution.is_empty()).then(|| (*attribution).to_owned()),
    })
}
