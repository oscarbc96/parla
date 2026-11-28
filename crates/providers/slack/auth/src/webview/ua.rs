//! Safari-shaped user agent. The sign-in page rejects WKWebView's default agent and
//! also a stale `Version/` token, so the version is derived from the OS.

/// Safari's major version for a macOS major version: Safari 26 shipped with macOS 26,
/// before that Safari 18 with macOS 15, 17 with 14, and so on (macOS + 3).
pub fn safari_version(os_major: u32) -> u32 {
    if os_major >= 26 {
        os_major
    } else {
        os_major + 3
    }
}

pub fn safari_user_agent(os_major: u32) -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{}.0 Safari/605.1.15",
        safari_version(os_major)
    )
}

/// Major macOS version from `sw_vers`, or 26 when it cannot be read (always
/// 26 off macOS, where `sw_vers` does not exist).
pub fn os_major() -> u32 {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().split('.').next().and_then(|m| m.parse().ok()))
        .unwrap_or(26)
}
