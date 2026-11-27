//! A tiny made-up provider for tests of the neutral engine and of the tools
//! built on it. Tokens are `fake-...`, hosts live under `chat.example.test`.

use regex::Regex;
use url::Url;

use crate::{HostSuffix, LOGIN_CODE, PATH_SLASH, RuleSet};

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern compiles")
}

/// Rules for the made-up provider `fake`.
pub struct FakeRules {
    tokens: Vec<Regex>,
    ids: Regex,
    suffixes: Vec<HostSuffix>,
    files: Regex,
    mention: Regex,
    slugs: Regex,
}

impl Default for FakeRules {
    fn default() -> Self {
        Self {
            tokens: vec![re(r"fake-[A-Za-z0-9][A-Za-z0-9%._-]*")],
            ids: re(r"\b[ABCDEFGH][A-Z0-9]{8,10}\b"),
            suffixes: vec![
                HostSuffix::new("chat.example.test")
                    .with_keep(&["app", "api", "ws", "org"])
                    .with_marker("org"),
            ],
            files: re(
                r#"(?i)(?:https?://|//)?[a-z0-9.-]*(?:cdn\.example\.test)(?:[/?#][^\s"'<>\\)\]]*)?"#,
            ),
            mention: re(r"^(?P<keep><@[^|>]*)(?:\|[^>]*)?(?P<tail>>)$"),
            slugs: re(&format!(
                r"(?i)(?P<pre>app\.chat\.example\.test{S}t{S})(?P<slug>[a-z0-9][a-z0-9-]*)(?:(?P<mid>{S}login{S})(?P<secret>{C}))?",
                S = PATH_SLASH,
                C = LOGIN_CODE,
            )),
        }
    }
}

const HEADER_PREFIX: &str = "x-chat-";

impl RuleSet for FakeRules {
    fn id(&self) -> &str {
        "fake"
    }
    fn tokens(&self) -> &[Regex] {
        &self.tokens
    }
    fn ids(&self) -> &Regex {
        &self.ids
    }
    fn host_suffixes(&self) -> &[HostSuffix] {
        &self.suffixes
    }
    fn file_hosts(&self) -> &Regex {
        &self.files
    }
    fn mention(&self) -> Option<&Regex> {
        Some(&self.mention)
    }
    fn path_slugs(&self) -> Option<&Regex> {
        Some(&self.slugs)
    }
    fn keep_header(&self, name: &str) -> bool {
        name.to_ascii_lowercase().starts_with(HEADER_PREFIX)
    }
    fn method_name(&self, url: &Url) -> Option<String> {
        let (_, method) = url.path().rsplit_once("/rpc/")?;
        (!method.is_empty()).then(|| method.to_string())
    }
}
