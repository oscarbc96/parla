use std::collections::BTreeSet;

use parla_plugin_sdk::Error;
use parla_plugin_sdk::host::LogLevel;
use parla_plugin_sdk::host::http::Response;
use parla_plugin_sdk::host::secrets;
use parla_plugin_sdk::media::{ContentRating, MediaItem, MediaPage};
use parla_plugin_sdk::testing::FakeHost;

use super::*;

const SEARCH: &str = include_str!("../tests/fixtures/search.json");
const TRENDING_1: &str = include_str!("../tests/fixtures/trending-page-1.json");
const TRENDING_2: &str = include_str!("../tests/fixtures/trending-page-2.json");
const RESOLVE: &str = include_str!("../tests/fixtures/resolve.json");
const EMPTY: &str =
    r#"{"data":[],"pagination":{"total_count":0,"count":0,"offset":0},"meta":{"status":200}}"#;

const KEY: &str = "synthetic-test-key";

fn response(status: u16, body: &str) -> Response {
    Response {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn fake() -> FakeHost {
    let fake = FakeHost::from_manifest(include_str!("../plugin.toml"));
    secrets::secret_set("api_key", KEY).unwrap();
    fake
}

fn fake_without_key() -> FakeHost {
    FakeHost::from_manifest(include_str!("../plugin.toml"))
}

fn search(
    query: &str,
    cursor: Option<&str>,
    limit: u32,
    rating: ContentRating,
) -> Result<MediaPage, Error> {
    Giphy::media_search(
        "giphy".into(),
        query.into(),
        cursor.map(str::to_string),
        limit,
        rating,
    )
}

fn trending(cursor: Option<&str>, limit: u32, rating: ContentRating) -> Result<MediaPage, Error> {
    Giphy::media_trending("giphy".into(), cursor.map(str::to_string), limit, rating)
}

fn resolve(id: &str) -> Result<MediaItem, Error> {
    Giphy::media_resolve("giphy".into(), id.into())
}

fn last_url(fake: &FakeHost) -> String {
    fake.http_requests().last().expect("no request").url.clone()
}

fn manifest() -> toml::Table {
    include_str!("../plugin.toml")
        .parse()
        .expect("plugin.toml is not valid TOML")
}

fn manifest_hosts() -> BTreeSet<String> {
    manifest()["capabilities"]["network"]
        .as_array()
        .expect("network is a list")
        .iter()
        .map(|h| h.as_str().expect("host is a string").to_string())
        .collect()
}

fn host_of(url: &str) -> &str {
    url.strip_prefix("https://")
        .expect("https URL")
        .split('/')
        .next()
        .expect("a host")
}

#[test]
fn search_maps_items() {
    let fake = fake();
    fake.on_http(|_| Ok(response(200, SEARCH)));

    let page = search("hello", None, 24, ContentRating::Unrated).unwrap();

    assert_eq!(page.items.len(), 2);
    let first = &page.items[0];
    assert_eq!(first.id, "synthAlpha01");
    assert_eq!(
        first.preview_url,
        "https://media.giphy.com/media/synthAlpha01/100w.gif"
    );
    assert_eq!(
        first.full_url,
        "https://media0.giphy.com/media/synthAlpha01/giphy.gif"
    );
    assert_eq!(first.mime_type, "image/gif");
    assert_eq!(
        (first.width, first.height, first.bytes),
        (480, 340, 180_000)
    );
    assert_eq!(first.title, "Synthetic hello");
    assert_eq!(first.rating, ContentRating::G);
    assert_eq!(first.attribution.as_deref(), Some("Powered by GIPHY"));

    let second = &page.items[1];
    assert_eq!(
        second.preview_url,
        "https://media2.giphy.com/media/synthBeta02/200w.gif"
    );
    assert_eq!(
        second.full_url,
        "https://i.giphy.com/media/synthBeta02/downsized.gif"
    );
    assert_eq!(
        (second.width, second.height, second.bytes),
        (300, 205, 95_000)
    );
    assert_eq!(second.title, "Synthetic beta alt");
    assert_eq!(second.rating, ContentRating::Pg13);
    assert_eq!(page.next.as_deref(), Some("3"));
}

#[test]
fn search_sends_key_query_limit_and_rating() {
    let fake = fake();
    fake.on_http(|_| Ok(response(200, EMPTY)));

    search("hello world", None, 7, ContentRating::Pg13).unwrap();

    let url = last_url(&fake);
    assert!(url.starts_with("https://api.giphy.com/v1/gifs/search?"));
    assert!(url.contains("api_key=synthetic-test-key"));
    assert!(url.contains("q=hello%20world"));
    assert!(url.contains("limit=7"));
    assert!(url.contains("offset=0"));
    assert!(url.contains("rating=pg-13"));
}

#[test]
fn trending_pages_with_cursor() {
    let fake = fake();
    fake.on_http(|request| {
        let body = if request.url.contains("offset=1") {
            TRENDING_2
        } else {
            TRENDING_1
        };
        Ok(response(200, body))
    });

    let first = trending(None, 1, ContentRating::Pg).unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.next.as_deref(), Some("1"));

    let second = trending(Some("1"), 1, ContentRating::Pg).unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.next, None);

    let requests = fake.http_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].url.contains("offset=0"));
    assert!(requests[0].url.contains("rating=pg"));
    assert!(requests[1].url.contains("offset=1"));
    assert!(requests[1].url.contains("rating=pg"));
}

#[test]
fn last_page_has_no_cursor() {
    let fake = fake();
    fake.on_http(|_| Ok(response(200, TRENDING_2)));

    let page = trending(Some("1"), 1, ContentRating::Unrated).unwrap();

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.next, None);
}

#[test]
fn missing_key_is_setup() {
    let fake = fake_without_key();
    fake.on_http(|_| Ok(response(200, SEARCH)));

    let error = search("hello", None, 10, ContentRating::Unrated).unwrap_err();

    assert!(
        matches!(error, Error::Invalid(ref message) if message == "api key"),
        "unexpected error: {error:?}"
    );
    assert!(fake.http_requests().is_empty());
}

#[test]
fn empty_key_is_setup() {
    let fake = fake_without_key();
    secrets::secret_set("api_key", "").unwrap();
    fake.on_http(|_| Ok(response(200, SEARCH)));

    let error = trending(None, 10, ContentRating::Unrated).unwrap_err();

    assert!(
        matches!(error, Error::Invalid(ref message) if message == "api key"),
        "unexpected error: {error:?}"
    );
    assert!(fake.http_requests().is_empty());
}

#[test]
fn unauthorized_is_setup() {
    for status in [401_u16, 403] {
        let fake = fake();
        fake.on_http(move |_| Ok(response(status, "{}")));

        let error = search("hello", None, 10, ContentRating::Unrated).unwrap_err();

        assert!(
            matches!(error, Error::Invalid(ref message) if message == "api key"),
            "status {status}: unexpected error: {error:?}"
        );
    }
}

#[test]
fn rate_limited_is_limit() {
    let fake = fake();
    fake.on_http(|_| Ok(response(429, "{}")));

    let error = search("hello", None, 10, ContentRating::Unrated).unwrap_err();

    assert!(
        matches!(error, Error::Limit(ref message) if message == "rate"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn bad_json_is_failed() {
    let fake = fake();
    fake.on_http(|_| Ok(response(200, "not json")));

    let error = search("hello", None, 10, ContentRating::Unrated).unwrap_err();

    assert!(
        matches!(error, Error::Failed(ref message) if message == "Giphy returned 200"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn server_error_is_failed() {
    let fake = fake();
    fake.on_http(|_| Ok(response(500, "{}")));

    let error = trending(None, 10, ContentRating::Unrated).unwrap_err();

    assert!(
        matches!(error, Error::Failed(ref message) if message == "Giphy returned 500"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn offline_passes_through() {
    let fake = fake();
    fake.inject("http.fetch", Error::Offline);

    let error = search("hello", None, 10, ContentRating::Unrated).unwrap_err();

    assert!(
        matches!(error, Error::Offline),
        "unexpected error: {error:?}"
    );
}

#[test]
fn item_without_size_is_skipped() {
    let fake = fake();
    fake.on_http(|_| Ok(response(200, SEARCH)));

    let page = search("hello", None, 24, ContentRating::Unrated).unwrap();

    assert_eq!(page.items.len(), 2);
    assert!(page.items.iter().all(|item| item.id != "synthGamma03"));
}

#[test]
fn key_never_logged() {
    {
        let fake = fake();
        fake.set_dev_mode(true);
        fake.on_http(|_| Ok(response(200, SEARCH)));

        search("hello", None, 10, ContentRating::Unrated).unwrap();

        let logs = fake.logs();
        assert!(!logs.is_empty(), "the plugin logs nothing");
        assert!(
            logs.iter().any(|(level, message)| *level == LogLevel::Debug
                && message.contains("/v1/gifs/search")
                && message.contains("200")),
            "no path and status log: {logs:?}"
        );
        assert!(
            logs.iter().all(|(_, message)| !message.contains(KEY)),
            "the API key leaked into the log: {logs:?}"
        );
    }

    {
        let fake = fake();
        fake.set_dev_mode(true);
        fake.on_http(|_| Ok(response(401, "{}")));

        let error = search("hello", None, 10, ContentRating::Unrated).unwrap_err();

        assert!(!format!("{error:?}").contains(KEY));
        assert!(
            fake.logs()
                .iter()
                .all(|(_, message)| !message.contains(KEY)),
            "the API key leaked into the log: {:?}",
            fake.logs()
        );
    }
}

#[test]
fn resolve_maps_one_item() {
    let fake = fake();
    fake.on_http(|_| Ok(response(200, RESOLVE)));

    let item = resolve("synthAlpha01").unwrap();

    assert_eq!(item.id, "synthAlpha01");
    assert_eq!(
        item.full_url,
        "https://media0.giphy.com/media/synthAlpha01/giphy.gif"
    );
    assert_eq!((item.width, item.height, item.bytes), (480, 340, 180_000));
    assert_eq!(item.attribution.as_deref(), Some("Powered by GIPHY"));
    assert!(
        last_url(&fake).contains("/v1/gifs/synthAlpha01?"),
        "unexpected URL: {}",
        last_url(&fake)
    );
}

#[test]
fn resolve_absent_is_not_found() {
    let fake = fake();
    fake.on_http(|_| Ok(response(404, "{\"data\":[]}")));

    let error = resolve("synthMissing").unwrap_err();

    assert!(
        matches!(error, Error::NotFound),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_provider_is_not_found() {
    let fake = fake_without_key();

    assert!(matches!(
        Giphy::media_search(
            "not-giphy".into(),
            "hello".into(),
            None,
            10,
            ContentRating::Unrated
        ),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        Giphy::media_trending("not-giphy".into(), None, 10, ContentRating::Unrated),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        Giphy::media_resolve("not-giphy".into(), "id".into()),
        Err(Error::NotFound)
    ));
    assert!(fake.http_requests().is_empty());
}

#[test]
fn rating_map_covers_all_limits() {
    for (rating, expected) in [
        (ContentRating::G, "rating=g"),
        (ContentRating::Pg, "rating=pg"),
        (ContentRating::Pg13, "rating=pg-13"),
        (ContentRating::R, "rating=r"),
        (ContentRating::Unrated, "rating=r"),
    ] {
        let fake = fake();
        fake.on_http(|_| Ok(response(200, EMPTY)));

        search("hello", None, 10, rating).unwrap();

        assert!(
            last_url(&fake).contains(expected),
            "{rating:?}: {}",
            last_url(&fake)
        );
    }
}

#[test]
fn query_is_percent_encoded() {
    let fake = fake();
    fake.on_http(|_| Ok(response(200, EMPTY)));

    search("hello world &/é", None, 10, ContentRating::Unrated).unwrap();

    let url = last_url(&fake);
    assert!(url.contains("q=hello%20world%20%26%2F%C3%A9"), "{url}");
    assert!(!url.contains("hello world"));
}

#[test]
fn search_cuts_query_to_50_chars() {
    let fake = fake();
    fake.on_http(|_| Ok(response(200, EMPTY)));

    let long = "a".repeat(60);
    search(&long, None, 10, ContentRating::Unrated).unwrap();

    let url = last_url(&fake);
    let query = url
        .split("q=")
        .nth(1)
        .expect("q parameter")
        .split('&')
        .next()
        .expect("q value");
    assert_eq!(query.len(), 50);
}

#[test]
fn every_item_url_is_on_a_manifest_host() {
    let fake = fake();
    fake.on_http(|request| {
        let body = if request.url.contains("/v1/gifs/search") {
            SEARCH
        } else if request.url.contains("/v1/gifs/trending") {
            if request.url.contains("offset=1") {
                TRENDING_2
            } else {
                TRENDING_1
            }
        } else {
            RESOLVE
        };
        Ok(response(200, body))
    });

    let mut items = search("hello", None, 24, ContentRating::Unrated)
        .unwrap()
        .items;
    items.extend(trending(None, 1, ContentRating::Unrated).unwrap().items);
    items.extend(
        trending(Some("1"), 1, ContentRating::Unrated)
            .unwrap()
            .items,
    );
    items.push(resolve("synthAlpha01").unwrap());

    let hosts = manifest_hosts();
    assert!(!items.is_empty());
    for item in items {
        for url in [&item.preview_url, &item.full_url] {
            let host = host_of(url);
            assert!(
                hosts.contains(host),
                "{host} is not in the manifest network grant ({url})"
            );
        }
    }
}

#[test]
fn manifest_matches_host_expectations() {
    let manifest = manifest();

    assert_eq!(manifest["id"].as_str(), Some("giphy"));
    assert_eq!(manifest["name"].as_str(), Some("GIF search: Giphy"));
    assert_eq!(manifest["version"].as_str(), Some("0.1.0"));
    assert_eq!(manifest["api"].as_str(), Some("0.1"));
    assert_eq!(manifest["license"].as_str(), Some("Apache-2.0 OR MIT"));

    let categories: Vec<&str> = manifest["categories"]
        .as_array()
        .expect("categories")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(categories, ["media"]);

    let activation: Vec<&str> = manifest["activation"]
        .as_array()
        .expect("activation")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(activation, ["media:giphy"]);

    let providers = manifest["media_providers"].as_array().expect("providers");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0]["id"].as_str(), Some("giphy"));
    assert_eq!(providers[0]["title"].as_str(), Some("GIPHY"));
    let kinds: Vec<&str> = providers[0]["kinds"]
        .as_array()
        .expect("kinds")
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["gif"]);

    let settings = manifest["settings"].as_array().expect("settings");
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0]["key"].as_str(), Some("api_key"));
    assert_eq!(settings[0]["type"].as_str(), Some("secret"));
    assert_eq!(settings[0]["title"].as_str(), Some("API key"));
    assert!(
        settings[0]["description"]
            .as_str()
            .unwrap()
            .contains("developers.giphy.com")
    );
    assert!(settings[0].get("default").is_none());

    let hosts = manifest_hosts();
    for host in [
        "api.giphy.com",
        "media.giphy.com",
        "media0.giphy.com",
        "media1.giphy.com",
        "media2.giphy.com",
        "media3.giphy.com",
        "media4.giphy.com",
        "i.giphy.com",
    ] {
        assert!(hosts.contains(host), "missing network host {host}");
    }
    assert_eq!(hosts.len(), 8);
    assert!(hosts.iter().all(|host| !host.contains('*')));
}
