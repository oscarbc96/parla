//! Renderer link patterns: `host/path`, the host
//! literal, `*` matching within one path segment.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::{Host, Url};

use super::argv::glob;

/// A parsed `host/path` pattern.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UrlPattern {
    text: String,
    host: String,
    segments: Vec<String>,
}

impl UrlPattern {
    /// `pattern` as the manifest wrote it; `None` without a path, with a
    /// scheme, or with a `*`, port or user in the host.
    pub fn new(pattern: &str) -> Option<Self> {
        if pattern.contains("://") {
            return None;
        }
        let (host, path) = pattern.split_once('/')?;
        if host.is_empty() || host.contains(['*', ':', '@', '?', '#']) {
            return None;
        }
        Some(Self {
            text: pattern.to_owned(),
            host: host.to_ascii_lowercase(),
            segments: path.split('/').map(str::to_owned).collect(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether `url` is a web link to this host whose path has the same
    /// segments, each matching. Query and fragment are not part of it.
    pub fn matches(&self, url: &Url) -> bool {
        if !matches!(url.scheme(), "https" | "http")
            || url.port().is_some()
            || !url.username().is_empty()
        {
            return false;
        }
        let Some(Host::Domain(host)) = url.host() else {
            return false;
        };
        if host.trim_end_matches('.') != self.host {
            return false;
        }
        let path = url.path().strip_prefix('/').unwrap_or(url.path());
        let segments: Vec<&str> = path.split('/').collect();
        segments.len() == self.segments.len()
            && self
                .segments
                .iter()
                .zip(segments)
                .all(|(p, s)| glob(p, s, &[]))
    }
}

impl Serialize for UrlPattern {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for UrlPattern {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let p = String::deserialize(d)?;
        Self::new(&p)
            .ok_or_else(|| serde::de::Error::custom(format_args!("`{p}` is not a link pattern")))
    }
}
