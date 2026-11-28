//! Media providers for the GIF picker: the neutral item and page types, the
//! rating setting, and the guest calls through the `media-provider` export.
//!
//! Everything that comes back is validated here before the picker sees it.
//! An invalid item is dropped, never a whole page; the only failures are the
//! deadline, a crash and the guest's own error.

use std::time::Duration;

use url::{Host, Url};
use wasmtime::component::Val;

use crate::caps::Grant;
use crate::engine::{CallError, PluginEngine, PluginInstance};

/// Deadline of one `media-provider` call.
pub const MEDIA_DEADLINE: Duration = Duration::from_secs(5);

/// Longest `query`, in bytes; longer is cut on a char boundary.
pub const MAX_QUERY_BYTES: usize = 256;
/// `limit` is clamped to this range.
pub const LIMIT_RANGE: std::ops::RangeInclusive<u32> = 1..=50;
/// Most items one page keeps.
pub const MAX_ITEMS: usize = 50;

const MAX_ID_BYTES: usize = 256;
const MAX_TITLE_BYTES: usize = 256;
const MAX_ATTRIBUTION_BYTES: usize = 128;
const INTERFACE: &str = "parla:plugin/media-provider@0.1.0";

/// A picker item: what it shows, where the bytes come from and how it is
/// rated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaItem {
    pub id: String,
    pub preview_url: String,
    pub full_url: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub title: String,
    pub rating: ContentRating,
    pub attribution: Option<String>,
}

/// One page of results; `next` is the guest's cursor for the following one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaPage {
    pub items: Vec<MediaItem>,
    pub next: Option<String>,
}

/// The user's content limit, in the picker's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContentRating {
    G,
    Pg,
    #[default]
    Pg13,
    R,
    /// The source gave no rating; kept whatever the limit is.
    Unrated,
}

impl ContentRating {
    /// The `content_rating` setting's spelling; anything unknown is `pg-13`.
    pub fn from_setting(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "g" => Self::G,
            "pg" => Self::Pg,
            "pg-13" | "pg13" => Self::Pg13,
            "r" => Self::R,
            _ => Self::Pg13,
        }
    }

    /// The WIT enum case.
    fn wit(self) -> &'static str {
        match self {
            Self::G => "g",
            Self::Pg => "pg",
            Self::Pg13 => "pg13",
            Self::R => "r",
            Self::Unrated => "unrated",
        }
    }

    fn from_wit(name: &str) -> Option<Self> {
        Some(match name {
            "g" => Self::G,
            "pg" => Self::Pg,
            "pg13" => Self::Pg13,
            "r" => Self::R,
            "unrated" => Self::Unrated,
            _ => return None,
        })
    }

    /// Position in the scale; [`Unrated`](Self::Unrated) ranks with `g` and
    /// is never filtered out.
    fn rank(self) -> u8 {
        match self {
            Self::G | Self::Unrated => 0,
            Self::Pg => 1,
            Self::Pg13 => 2,
            Self::R => 3,
        }
    }
}

/// Why a media call failed. [`Invalid`](Self::Invalid) stays apart because
/// the picker shows "needs set-up" for `invalid("api key")`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("denied: {0}")]
    Denied(String),
    #[error("offline")]
    Offline,
    #[error("limit: {0}")]
    Limit(String),
    #[error("failed: {0}")]
    Failed(String),
    #[error("the provider ran past its deadline")]
    Deadline,
    #[error("the provider crashed")]
    Crashed,
}

impl From<CallError> for MediaError {
    fn from(e: CallError) -> Self {
        match e {
            CallError::Deadline => Self::Deadline,
            _ => Self::Crashed,
        }
    }
}

/// `media-search` for `provider_id`, with `query` cut to 256 bytes and
/// `limit` clamped to 1..=50.
pub async fn search(
    engine: &PluginEngine,
    instance: &mut PluginInstance,
    provider_id: &str,
    query: &str,
    cursor: Option<&str>,
    limit: u32,
    max_rating: ContentRating,
) -> Result<MediaPage, MediaError> {
    let params = vec![
        Val::String(provider_id.to_owned()),
        Val::String(cut(query, MAX_QUERY_BYTES).to_owned()),
        cursor_val(cursor),
        Val::U32(clamp_limit(limit)),
        Val::Enum(max_rating.wit().to_owned()),
    ];
    let grant = instance.grant().clone();
    let page = call(engine, instance, "media-search", params).await?;
    page_from_val(&page, &grant, max_rating)
}

/// `media-trending`, the same validation and limits as [`search`].
pub async fn trending(
    engine: &PluginEngine,
    instance: &mut PluginInstance,
    provider_id: &str,
    cursor: Option<&str>,
    limit: u32,
    max_rating: ContentRating,
) -> Result<MediaPage, MediaError> {
    let params = vec![
        Val::String(provider_id.to_owned()),
        cursor_val(cursor),
        Val::U32(clamp_limit(limit)),
        Val::Enum(max_rating.wit().to_owned()),
    ];
    let grant = instance.grant().clone();
    let page = call(engine, instance, "media-trending", params).await?;
    page_from_val(&page, &grant, max_rating)
}

/// `media-resolve`: the one item for `item_id`, validated the same way.
pub async fn resolve(
    engine: &PluginEngine,
    instance: &mut PluginInstance,
    provider_id: &str,
    item_id: &str,
) -> Result<MediaItem, MediaError> {
    let params = vec![
        Val::String(provider_id.to_owned()),
        Val::String(item_id.to_owned()),
    ];
    let grant = instance.grant().clone();
    let item = call(engine, instance, "media-resolve", params).await?;
    item_from_val(&item, &grant, None)
        .ok_or_else(|| MediaError::Failed("the provider returned an invalid item".to_owned()))
}

async fn call(
    engine: &PluginEngine,
    instance: &mut PluginInstance,
    name: &str,
    params: Vec<Val>,
) -> Result<Val, MediaError> {
    let func = instance.export_func(INTERFACE, name).ok_or_else(|| {
        MediaError::Failed("the plugin does not export media-provider".to_owned())
    })?;
    let mut results = [Val::Result(Err(None))];
    engine
        .call(instance, func, &params, &mut results, MEDIA_DEADLINE)
        .await?;
    match results {
        [Val::Result(Ok(Some(v)))] => Ok(*v),
        [Val::Result(Err(e))] => Err(error_from_val(e.as_deref())),
        [other] => Err(MediaError::Failed(format!(
            "unexpected media-provider result {other:?}"
        ))),
    }
}

fn error_from_val(error: Option<&Val>) -> MediaError {
    let Some(Val::Variant(name, payload)) = error else {
        return MediaError::Failed("the provider returned an error".to_owned());
    };
    let text = payload
        .as_deref()
        .and_then(as_str)
        .unwrap_or_default()
        .to_owned();
    match name.as_str() {
        "invalid" => MediaError::Invalid(text),
        "denied" => MediaError::Denied(text),
        "offline" => MediaError::Offline,
        "limit" => MediaError::Limit(text),
        "failed" => MediaError::Failed(text),
        "unsupported" => MediaError::Failed("unsupported".to_owned()),
        "not-found" => MediaError::Failed("not found".to_owned()),
        "cancelled" => MediaError::Failed("cancelled".to_owned()),
        other => MediaError::Failed(other.to_owned()),
    }
}

fn page_from_val(
    value: &Val,
    grant: &Grant,
    max_rating: ContentRating,
) -> Result<MediaPage, MediaError> {
    let Val::Record(fields) = value else {
        return Err(MediaError::Failed(
            "the provider returned no page".to_owned(),
        ));
    };
    let items = match field(fields, "items") {
        Some(Val::List(items)) => items,
        _ => return Err(MediaError::Failed("the page has no items".to_owned())),
    };
    let items = items
        .iter()
        .filter_map(|item| item_from_val(item, grant, Some(max_rating)))
        .take(MAX_ITEMS)
        .collect();
    let next = match field(fields, "next") {
        Some(Val::Option(Some(cursor))) => as_str(cursor).map(str::to_owned),
        _ => None,
    };
    Ok(MediaPage { items, next })
}

/// One item, `None` when any rule fails (the item is then dropped).
fn item_from_val(
    value: &Val,
    grant: &Grant,
    max_rating: Option<ContentRating>,
) -> Option<MediaItem> {
    let Val::Record(fields) = value else {
        return None;
    };
    let id = as_str(field(fields, "id")?)?.to_owned();
    if id.is_empty() || id.len() > MAX_ID_BYTES {
        return None;
    }
    let preview_url = as_str(field(fields, "preview-url")?)?.to_owned();
    let full_url = as_str(field(fields, "full-url")?)?.to_owned();
    if !on_grant(grant, &preview_url) || !on_grant(grant, &full_url) {
        return None;
    }
    let width = as_u32(field(fields, "width")?)?;
    let height = as_u32(field(fields, "height")?)?;
    if width == 0 || height == 0 {
        return None;
    }
    let rating = ContentRating::from_wit(as_enum(field(fields, "rating")?)?)?;
    if let Some(limit) = max_rating
        && rating != ContentRating::Unrated
        && rating.rank() > limit.rank()
    {
        return None;
    }
    let mime_type = as_str(field(fields, "mime-type")?)?.to_owned();
    let bytes = as_u64(field(fields, "bytes")?)?;
    let title = cut(as_str(field(fields, "title")?)?, MAX_TITLE_BYTES).to_owned();
    let attribution = match field(fields, "attribution") {
        Some(Val::Option(Some(v))) => Some(cut(as_str(v)?, MAX_ATTRIBUTION_BYTES).to_owned()),
        _ => None,
    };
    Some(MediaItem {
        id,
        preview_url,
        full_url,
        mime_type,
        width,
        height,
        bytes,
        title,
        rating,
        attribution,
    })
}

fn field<'a>(fields: &'a [(String, Val)], name: &str) -> Option<&'a Val> {
    fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn as_str(value: &Val) -> Option<&str> {
    match value {
        Val::String(s) => Some(s),
        _ => None,
    }
}

fn as_u32(value: &Val) -> Option<u32> {
    match value {
        Val::U32(n) => Some(*n),
        _ => None,
    }
}

fn as_u64(value: &Val) -> Option<u64> {
    match value {
        Val::U64(n) => Some(*n),
        _ => None,
    }
}

fn as_enum(value: &Val) -> Option<&str> {
    match value {
        Val::Enum(name) => Some(name),
        _ => None,
    }
}

/// Both URLs must be https on a named host the grant covers.
fn on_grant(grant: &Grant, url: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    match url.host() {
        Some(Host::Domain(host)) => grant.network.iter().any(|h| h.matches(host)),
        _ => false,
    }
}

fn cursor_val(cursor: Option<&str>) -> Val {
    Val::Option(cursor.map(|c| Box::new(Val::String(c.to_owned()))))
}

fn clamp_limit(limit: u32) -> u32 {
    limit.clamp(*LIMIT_RANGE.start(), *LIMIT_RANGE.end())
}

/// `text` cut to `max` bytes on a char boundary.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_map_to_ratings() {
        assert_eq!(ContentRating::from_setting("g"), ContentRating::G);
        assert_eq!(ContentRating::from_setting("PG"), ContentRating::Pg);
        assert_eq!(ContentRating::from_setting("pg-13"), ContentRating::Pg13);
        assert_eq!(ContentRating::from_setting("pg13"), ContentRating::Pg13);
        assert_eq!(ContentRating::from_setting("r"), ContentRating::R);
        assert_eq!(ContentRating::from_setting("nonsense"), ContentRating::Pg13);
    }

    #[test]
    fn limits_are_clamped() {
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(10), 10);
        assert_eq!(clamp_limit(500), 50);
    }

    #[test]
    fn cutting_respects_char_boundaries() {
        let text = "aaaaé";
        assert_eq!(cut(text, 5), "aaaa");
        assert_eq!(cut(text, 6), text);
        assert_eq!(cut(text, 100), text);
    }
}
