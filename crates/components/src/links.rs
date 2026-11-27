//! Link safety: only a short allowlist of schemes ever reaches the OS.

/// What to do with a link the user clicked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    OpenExternal(String),
    Refuse,
}

/// Allows `https`, `http` and `mailto` (case-insensitive scheme); everything else is refused.
pub fn open_link(url: &str) -> LinkAction {
    let url = url.trim();
    let Some((scheme, rest)) = url.split_once(':') else {
        return LinkAction::Refuse;
    };
    let ok = match scheme.to_ascii_lowercase().as_str() {
        "https" | "http" => rest.starts_with("//") && rest.len() > 2,
        "mailto" => !rest.is_empty(),
        _ => false,
    };
    if ok {
        LinkAction::OpenExternal(url.to_string())
    } else {
        LinkAction::Refuse
    }
}
