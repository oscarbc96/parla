//! The fallback ladder: which routes a join may try, in order.

use settings::JoinWith;

/// One way to open a huddle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinRoute {
    /// Native audio in Parla (no window).
    Native,
    /// The call window (a helper process with a webview).
    Window,
    /// The default browser, at this URL.
    Browser(String),
    /// The provider's own app, at this URL.
    ProviderApp(String),
}

/// Routes to try for `join_with`, best first. A route is present only when
/// available: `Native` needs `native_ok`, `Window` needs `window_ok`, the
/// others need their URL.
///
/// `Native` falls to the window, then the browser, then the app; `Window`
/// falls to the browser, then the app (never native); `Browser` and
/// `ProviderApp` never try the window and fall to each other.
pub fn ladder(
    join_with: JoinWith,
    native_ok: bool,
    window_ok: bool,
    web: Option<String>,
    app: Option<String>,
) -> Vec<JoinRoute> {
    let browser = web.map(JoinRoute::Browser);
    let app = app.map(JoinRoute::ProviderApp);
    let window = window_ok.then_some(JoinRoute::Window);
    let native = native_ok.then_some(JoinRoute::Native);
    let order: [Option<JoinRoute>; 4] = match join_with {
        JoinWith::Native => [native, window, browser, app],
        JoinWith::Window => [None, window, browser, app],
        JoinWith::Browser => [None, None, browser, app],
        JoinWith::ProviderApp => [None, None, app, browser],
    };
    order.into_iter().flatten().collect()
}
