//! Media providers through the host: `media-provider` calls a real guest, the
//! guest calls `http.fetch` and `secrets.secret-get`, and the host validates
//! what comes back before the picker sees it.

mod common;

use std::sync::Arc;

use plugin_host::backend::MemoryBackend;
use plugin_host::caps::Grant;
use plugin_host::engine::{PluginEngine, PluginInstance};
use plugin_host::manifest::Manifest;
use plugin_host::media::{self, ContentRating, MediaError};

/// A media provider manifest; `network` is what its card grants.
fn manifest(id: &str, network: &[&str]) -> Arc<Manifest> {
    let network = network
        .iter()
        .map(|h| format!("\"{h}\""))
        .collect::<Vec<_>>()
        .join(", ");
    common::manifest(&format!(
        r#"id = "{id}"
name = "Test GIF"
version = "0.1.0"
api = "0.1"

[capabilities]
network = [{network}]

[[media_providers]]
id = "gifs"
title = "GIFs"
kinds = ["gif"]

[[settings]]
key = "page_size"
type = "integer"
default = 20
title = "Page size"
"#
    ))
}

async fn instance(
    engine: &PluginEngine,
    backend: Arc<MemoryBackend>,
    id: &str,
    network: &[&str],
) -> PluginInstance {
    let m = manifest(id, network);
    let grant = Grant::from_manifest(&m);
    common::instantiate(
        engine,
        &common::guest("media"),
        common::context(m, grant, backend),
    )
    .await
}

/// One line of the guest's body format.
fn item(id: &str, preview: &str, full: &str, rating: &str) -> String {
    format!("{id}|{preview}|{full}|image/gif|10|10|1000|A cat|{rating}|Example")
}

fn good(id: &str) -> String {
    item(
        id,
        &format!("https://api.example.test/{id}.gif"),
        &format!("https://api.example.test/{id}-full.gif"),
        "g",
    )
}

#[test]
fn search_returns_items_from_granted_hosts() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        backend.script(200, Vec::new(), good("a").into_bytes());
        let mut plugin =
            instance(&engine, backend.clone(), "gif-test", &["api.example.test"]).await;
        let page = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap();
        assert_eq!(page.items.len(), 1, "{:?}", page.items);
        let got = &page.items[0];
        assert_eq!(got.id, "a");
        assert_eq!(got.preview_url, "https://api.example.test/a.gif");
        assert_eq!(got.full_url, "https://api.example.test/a-full.gif");
        assert_eq!(got.mime_type, "image/gif");
        assert_eq!((got.width, got.height, got.bytes), (10, 10, 1000));
        assert_eq!(got.title, "A cat");
        assert_eq!(got.rating, ContentRating::G);
        assert_eq!(got.attribution.as_deref(), Some("Example"));
        let req = &backend.requests()[0];
        assert!(req.url.contains("q=cats"), "{}", req.url);
        assert_eq!(req.headers[0].1, "secret");
    });
}

#[test]
fn item_url_off_card_is_dropped() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        let mut body = good("a");
        body.push('\n');
        body.push_str(&item(
            "b",
            "https://api.example.test/b.gif",
            "https://evil.example.test/b-full.gif",
            "g",
        ));
        backend.script(200, Vec::new(), body.into_bytes());
        let mut plugin = instance(&engine, backend, "gif-test", &["api.example.test"]).await;
        let page = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap();
        assert_eq!(page.items.len(), 1, "{:?}", page.items);
        assert_eq!(page.items[0].id, "a");
    });
}

#[test]
fn rating_sent_with_query() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        backend.script(200, Vec::new(), Vec::new());
        let mut plugin =
            instance(&engine, backend.clone(), "gif-test", &["api.example.test"]).await;
        media::search(
            &engine,
            &mut plugin,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap();
        let url = &backend.requests()[0].url;
        assert!(url.contains("rating=pg13"), "{url}");
    });
}

#[test]
fn item_above_rating_is_dropped() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        let mut body = item(
            "r",
            "https://api.example.test/r.gif",
            "https://api.example.test/r-full.gif",
            "r",
        );
        body.push('\n');
        body.push_str(&item(
            "u",
            "https://api.example.test/u.gif",
            "https://api.example.test/u-full.gif",
            "unrated",
        ));
        backend.script(200, Vec::new(), body.into_bytes());
        let mut plugin = instance(&engine, backend, "gif-test", &["api.example.test"]).await;
        let page = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg,
        )
        .await
        .unwrap();
        let ids: Vec<&str> = page.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["u"], "{:?}", page.items);
    });
}

#[test]
fn missing_key_is_invalid_api_key() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        let mut plugin = instance(&engine, backend, "gif-test", &["api.example.test"]).await;
        let err = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        match err {
            MediaError::Invalid(why) => assert_eq!(why, "api key"),
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn deadline_returns_error() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        let mut plugin = instance(&engine, backend, "gif-test", &["api.example.test"]).await;
        let err = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "sleep",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, MediaError::Deadline), "{err:?}");
    });
}

#[test]
fn fetch_to_ungranted_host_is_denied() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        let mut plugin = instance(&engine, backend.clone(), "gif-test", &[]).await;
        let err = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, MediaError::Denied(why) if why.contains("api.example.test")),
            "{err:?}"
        );
        assert!(backend.requests().is_empty(), "the backend was called");
    });
}

#[test]
fn redirect_to_ungranted_host_is_denied() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        backend.script(
            302,
            vec![("Location".into(), "https://evil.example.test/x".into())],
            Vec::new(),
        );
        let mut plugin = instance(&engine, backend, "gif-test", &["api.example.test"]).await;
        let err = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "redirect",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, MediaError::Denied(why) if why.contains("evil.example.test")),
            "{err:?}"
        );
    });
}

#[test]
fn http_is_refused() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        let mut plugin =
            instance(&engine, backend.clone(), "gif-test", &["api.example.test"]).await;
        let err = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "http",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, MediaError::Invalid(why) if why.contains("https")),
            "{err:?}"
        );
        assert!(backend.requests().is_empty(), "the backend was called");
    });
}

#[test]
fn forbidden_request_header_is_invalid() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        let mut plugin =
            instance(&engine, backend.clone(), "gif-test", &["api.example.test"]).await;
        let err = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "header",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, MediaError::Invalid(why) if why.to_lowercase().contains("cookie")),
            "{err:?}"
        );
        assert!(backend.requests().is_empty(), "the backend was called");
    });
}

#[test]
fn response_over_cap_is_limit() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        // 16 MiB of guest memory: a quarter of it is 4 MiB.
        backend.script(200, Vec::new(), vec![b'x'; (4 << 20) + 1]);
        let mut plugin = instance(&engine, backend, "gif-test", &["api.example.test"]).await;
        let err = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        match err {
            MediaError::Limit(what) => assert_eq!(what, "response body"),
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn secret_is_namespaced_by_plugin() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        backend.set_secret("gif-test", "api_key", "secret");
        backend.script(200, Vec::new(), good("a").into_bytes());
        backend.script(200, Vec::new(), good("b").into_bytes());
        let mut first = instance(&engine, backend.clone(), "gif-test", &["api.example.test"]).await;
        media::search(
            &engine,
            &mut first,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap();
        let mut other =
            instance(&engine, backend.clone(), "gif-other", &["api.example.test"]).await;
        let err = media::search(
            &engine,
            &mut other,
            "gifs",
            "cats",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, MediaError::Invalid(ref why) if why == "api key"),
            "{err:?}"
        );
    });
}

#[test]
fn undeclared_setting_is_not_found() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let backend = common::memory_backend();
        let mut plugin = instance(&engine, backend, "gif-test", &["api.example.test"]).await;
        let err = media::search(
            &engine,
            &mut plugin,
            "gifs",
            "setting:nope",
            None,
            10,
            ContentRating::Pg13,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, MediaError::Failed(why) if why.contains("not found")),
            "{err:?}"
        );
    });
}
