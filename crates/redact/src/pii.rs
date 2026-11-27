//! Key rules for JSON, form and query values, and the `Redactor` entry points.

use std::sync::Arc;

use serde_json::Value;

use crate::Redactor;
use crate::patterns::SECRET_PLACEHOLDER;

const LOREM: [&str; 8] = [
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "elit",
    "sed",
];

enum Rule {
    Secret,
    Email,
    /// Replace with `"{label} {n}"`.
    Label(&'static str),
    /// Message-like text: same word count, mentions kept.
    Text,
    /// Avatar and thumbnail URLs.
    Image,
    /// Permalinks and file download URLs.
    FileUrl,
    /// Workspace subdomain.
    Domain,
}

/// Keys whose string value is free text and becomes lorem ipsum.
const TEXT_KEYS: [&str; 24] = [
    "text",
    "fallback",
    "pretext",
    "preview",
    "title",
    "description",
    "comment",
    "initial_comment",
    "status_text",
    "message",
    "caption",
    "topic",
    "purpose",
    "footer",
    "author_name",
    "author_subname",
    "service_name",
    "alt_text",
    "alt_txt",
    "subtitle",
    "label",
    "placeholder",
    "hint",
    "summary",
];

/// Keys of link metadata where even a short value (a service or site name) is real.
pub(crate) fn is_strict_text_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "footer" | "author_name" | "author_subname" | "service_name"
    )
}

/// One-time login codes and the URLs that embed them (`magic_login_code`, ...).
pub(crate) fn is_magic_login_key(key: &str) -> bool {
    key.to_ascii_lowercase().starts_with("magic_login")
}

/// An entity-encoded blob (`&quot;`...) smuggled through a generic key such as `value`.
pub(crate) fn is_encoded_blob(key: &str, value: &str) -> bool {
    key.eq_ignore_ascii_case("value") && (value.contains("&quot;") || value.contains("&amp;"))
}

pub(crate) fn is_text_key(key: &str) -> bool {
    TEXT_KEYS.contains(&key.to_ascii_lowercase().as_str())
}

/// Free text shorter than this is not worth flagging (labels, emoji names).
const LONG_TEXT: usize = 20;

/// True when `value` is short, or made of placeholder words and mentions only.
pub(crate) fn is_placeholder_text(value: &str, rules: &dyn crate::RuleSet) -> bool {
    is_placeholder(value, rules, LONG_TEXT)
}

pub(crate) fn is_placeholder(value: &str, rules: &dyn crate::RuleSet, min_len: usize) -> bool {
    // `<target|label>` entities: only the label is prose (targets are checked as URLs/IDs).
    static ENTITY: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>|]*(?:\|([^>]*))?>").expect("static"));
    let prose = ENTITY.replace_all(value, "$1");
    prose.len() < min_len
        || prose.split_whitespace().all(|w| {
            LOREM.contains(&w)
                || rules.mention().is_some_and(|re| re.is_match(w))
                || w.starts_with("https://example.com/")
        })
}

/// Keys whose value is a credential whatever it looks like. `-` and `_` are the same,
/// and plurals match (`tokens`, `cookies`).
pub(crate) fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase().replace('-', "_");
    let k = k.as_str();
    matches!(
        k,
        "frt"
            | "authorization"
            | "cookie"
            | "cookies"
            | "set_cookie"
            | "password"
            | "passwd"
            | "pwd"
            | "passphrase"
            | "sig"
            | "session"
            | "session_id"
            | "sessionid"
            | "credential"
            | "credentials"
            | "code"
            | "apikey"
    ) || k.ends_with("token")
        || k.ends_with("tokens")
        || k.ends_with("api_key")
        || k.ends_with("apikey")
        || k.ends_with("_cookie")
        || k.ends_with("_cookies")
        || k.ends_with("_signature")
        || k.ends_with("_sig")
        || k.ends_with("_password")
        || k.ends_with("_session")
        || k.contains("secret")
        || k.contains("signature")
}

fn rule_for(key: Option<&str>, parent: Option<&str>) -> Option<Rule> {
    let key = key?.to_ascii_lowercase();
    let parent = parent.map(str::to_ascii_lowercase);
    if key == "value" && matches!(parent.as_deref(), Some("topic" | "purpose")) {
        return Some(Rule::Text);
    }
    let k = key.as_str();
    if is_secret_key(k) || is_magic_login_key(k) {
        return Some(Rule::Secret);
    }
    if k == "email" || k.ends_with("_email") {
        return Some(Rule::Email);
    }
    if is_text_key(k) {
        return Some(Rule::Text);
    }
    if k.starts_with("image_")
        || k.starts_with("thumb_")
        || k.ends_with("_image")
        || k.contains("avatar_url")
    {
        return Some(Rule::Image);
    }
    if matches!(
        k,
        "permalink"
            | "permalink_public"
            | "url_private"
            | "url_private_download"
            | "url_download"
            | "download_url"
            | "upload_url"
    ) {
        return Some(Rule::FileUrl);
    }
    if matches!(
        k,
        "domain" | "team_domain" | "enterprise_domain" | "subdomain"
    ) {
        return Some(Rule::Domain);
    }
    let label = match k {
        "name"
        | "real_name"
        | "display_name"
        | "real_name_normalized"
        | "display_name_normalized"
        | "first_name"
        | "last_name"
        | "full_name"
        | "username"
        | "user_name"
        | "nickname" => "Name",
        "team_name" | "enterprise_name" => "Workspace",
        "phone" | "skype" => "Contact",
        "handle" => "Handle",
        "filename" | "file_name" => "File",
        "avatar_hash" => "Avatar",
        _ => return None,
    };
    Some(Rule::Label(label))
}

impl Redactor {
    /// Redact a JSON document in place: key rules, IDs in values and object keys,
    /// and JSON embedded in string values (`blocks`, `attachments`).
    pub fn redact_json(&mut self, value: &mut Value) {
        self.json_inner(value, None, None);
    }

    /// Redact decoded form (or query) parameters in place.
    pub fn redact_form(&mut self, params: &mut [(String, String)]) {
        for (key, value) in params.iter_mut() {
            *value = self.leaf(Some(key), None, value);
        }
    }

    /// Redact a non-JSON body or frame: secrets, IDs and URLs as in `redact_str`, and
    /// long free text becomes lorem ipsum (short text such as `ok` is kept).
    pub fn redact_prose(&mut self, text: &str) -> String {
        let out = self.redact_str(text);
        let rules = Arc::clone(&self.rules);
        if is_placeholder_text(&out, &*rules) {
            out
        } else {
            self.fake_text(&out)
        }
    }

    /// Redact a URL (absolute or `/path?query`): host, IDs and secrets in the path,
    /// key rules for query parameters.
    pub fn redact_url(&mut self, url: &str) -> String {
        let (rest, fragment) = match url.split_once('#') {
            Some((r, f)) => (r, Some(f)),
            None => (url, None),
        };
        let (path, query) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        let mut out = self.redact_str(path);
        if let Some(query) = query {
            let mut params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            self.redact_form(&mut params);
            if !params.is_empty() {
                let encoded = url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(&params)
                    .finish();
                out.push('?');
                out.push_str(&encoded);
            }
        }
        if let Some(fragment) = fragment {
            out.push('#');
            out.push_str(&self.redact_str(fragment));
        }
        out
    }

    fn json_inner(&mut self, value: &mut Value, key: Option<&str>, parent: Option<&str>) {
        if matches!(rule_for(key, parent), Some(Rule::Secret)) && !value.is_null() {
            *value = Value::String(SECRET_PLACEHOLDER.to_string());
            return;
        }
        match value {
            Value::Object(map) => {
                let old = std::mem::take(map);
                for (k, mut v) in old {
                    let new_key = self.redact_str(&k);
                    self.json_inner(&mut v, Some(&k), key);
                    map.insert(new_key, v);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.json_inner(item, key, parent);
                }
            }
            Value::String(s) => {
                let new = self.leaf(key, parent, s);
                *s = new;
            }
            _ => {}
        }
    }

    /// Redact one string value according to its key.
    fn leaf(&mut self, key: Option<&str>, parent: Option<&str>, value: &str) -> String {
        if value.is_empty() {
            return String::new();
        }
        if key.is_some_and(|k| is_encoded_blob(k, value)) {
            return self.fake_text(value);
        }
        match rule_for(key, parent) {
            Some(Rule::Secret) => SECRET_PLACEHOLDER.to_string(),
            Some(Rule::Email) => {
                if crate::patterns::is_fake_email(value) {
                    return value.to_string();
                }
                let n = self.map.fake_number("email", value);
                format!("user{n}@example.test")
            }
            Some(Rule::Label(label)) => {
                let n = self.map.fake_number(label, value);
                format!("{label} {n}")
            }
            Some(Rule::Text) => self.fake_text(value),
            Some(Rule::Image) => {
                let n = self.map.fake_number("image", value);
                format!("https://files.example.test/image/{n}")
            }
            Some(Rule::FileUrl) => {
                let n = self.map.fake_number("file", value);
                format!("https://files.example.test/file/{n}")
            }
            Some(Rule::Domain) => self.map.host_label(value),
            None => self.plain(value),
        }
    }

    /// A URL on an allowed host stays, but its query goes through the key rules, so
    /// `?api_key=..&sig=..` never survives.
    pub(crate) fn redact_allowed_url_query(&mut self, url: &str) -> String {
        let Some((base, rest)) = url.split_once('?') else {
            return url.to_string();
        };
        let (query, fragment) = match rest.split_once('#') {
            Some((q, f)) => (q, Some(f)),
            None => (rest, None),
        };
        let mut params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let before = params.clone();
        self.redact_form(&mut params);
        if params == before {
            return url.to_string();
        }
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&params)
            .finish();
        let mut out = format!("{base}?{encoded}");
        if let Some(f) = fragment {
            out.push('#');
            out.push_str(f);
        }
        out
    }

    /// No key rule: parse JSON embedded in a string, else apply the string rules.
    fn plain(&mut self, value: &str) -> String {
        let trimmed = value.trim_start();
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && let Ok(mut inner) = serde_json::from_str::<Value>(value)
        {
            self.redact_json(&mut inner);
            return inner.to_string();
        }
        self.redact_str(value)
    }

    /// Same word count as `text`; mentions (as defined by the rule set) keep their
    /// (mapped) ID so the shape of the message survives.
    fn fake_text(&mut self, text: &str) -> String {
        let rules = Arc::clone(&self.rules);
        let mut out = Vec::new();
        for (i, word) in text.split_whitespace().enumerate() {
            if let Some(caps) = rules.mention().and_then(|re| re.captures(word)) {
                let keep = caps.name("keep").map_or("", |m| m.as_str());
                let tail = caps.name("tail").map_or("", |m| m.as_str());
                out.push(format!("{}{tail}", self.redact_str(keep)));
            } else {
                out.push(LOREM[i % LOREM.len()].to_string());
            }
        }
        out.join(" ")
    }
}
