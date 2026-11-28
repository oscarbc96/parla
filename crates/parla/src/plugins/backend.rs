//! The app side of the plugin host API for the built-in providers: HTTPS to
//! the hosts the manifest grants, the plugin secrets store, the plugin's
//! `plugins.<id>.config` settings and the app info.
//!
//! `http_fetch` follows the [`HostBackend`] contract: https only, no
//! redirects (the host follows them itself, checking each host), no cookies,
//! `User-Agent: Parla-Plugin/<id>`, and the body read up to the cap it is
//! given. The transport is a small trait ([`Transport`]) so tests script
//! responses without the network.
//!
//! A name that resolves to a loopback, link-local, private or otherwise
//! non-public address is refused: [`HttpTransport`] installs a resolver
//! ([`PublicResolver`]) that drops those addresses before any connection is
//! made. Refusals and failures name the host and status only, never the URL,
//! path or query (the Klipy key is a path segment, the Giphy key a query
//! parameter).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use domain::SecretText;
use futures::future::BoxFuture;
use plugin_host::backend::{
    ApiError, AppInfo, HostBackend, HttpMethod, HttpRequest, HttpResponse, SettingValue,
};
use plugin_host::manifest::{Manifest, PluginId};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use settings::PluginsSettings;
use url::{Host, Url};

use super::secrets::Secrets;

/// `User-Agent` of every plugin request; the plugin id is appended.
const USER_AGENT: &str = "Parla-Plugin";

/// One HTTP request, from [`AppBackend`] to the network (or a test script).
pub trait Transport: Send + Sync + 'static {
    fn fetch<'a>(
        &'a self,
        plugin: &str,
        request: HttpRequest,
        max_body: usize,
    ) -> BoxFuture<'a, Result<HttpResponse, ApiError>>;
}

/// The real transport: one shared `reqwest::Client`, no cookies, no
/// redirects, the workspace's proxy and trust-store defaults, and a DNS
/// resolver that only hands out public addresses.
pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            // The host follows redirects itself, checking every new host
            // against the grant; this client never does.
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            // `true` is cancelled by neither side; the request's own timeout
            // bounds the whole exchange (`HttpRequest::timeout_ms`).
            .dns_resolver(PublicResolver)
            .build()
            .expect("static client configuration is valid");
        Self { client }
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for HttpTransport {
    fn fetch<'a>(
        &'a self,
        plugin: &str,
        request: HttpRequest,
        max_body: usize,
    ) -> BoxFuture<'a, Result<HttpResponse, ApiError>> {
        let plugin = plugin.to_owned();
        Box::pin(async move {
            let url = Url::parse(&request.url).map_err(|_| ApiError::Invalid("url".to_owned()))?;
            if url.scheme() != "https" {
                return Err(ApiError::Invalid("https".to_owned()));
            }
            let client = &self.client;
            let mut builder = match request.method {
                HttpMethod::Get => client.get(url),
                HttpMethod::Head => client.head(url),
                HttpMethod::Post => client.post(url),
                HttpMethod::Put => client.put(url),
                HttpMethod::Patch => client.patch(url),
                HttpMethod::Delete => client.delete(url),
            };
            for (name, value) in &request.headers {
                builder = builder.header(name, value);
            }
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            let builder = builder
                .header(
                    reqwest::header::USER_AGENT,
                    format!("{USER_AGENT}/{}", plugin),
                )
                .timeout(Duration::from_millis(u64::from(request.timeout_ms.max(1))));
            let response = builder.send().await.map_err(transport_error)?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect();
            if response
                .content_length()
                .is_some_and(|len| len > max_body as u64)
            {
                return Err(ApiError::Limit("response body".to_owned()));
            }
            let mut body = Vec::new();
            let mut response = response;
            while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
                if body.len() + chunk.len() > max_body {
                    return Err(ApiError::Limit("response body".to_owned()));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

/// A failed exchange: offline when the name did not resolve or the connection
/// timed out or failed, otherwise a generic failure. The URL is always
/// dropped: it may carry the API key.
fn transport_error(e: reqwest::Error) -> ApiError {
    if e.is_timeout() || e.is_connect() || e.is_dns() {
        ApiError::Offline
    } else {
        ApiError::Failed(e.without_url().to_string())
    }
}

/// Resolves a manifest host and refuses every non-public address. A host
/// that resolves to any loopback, link-local or private address fails, so a
/// rebound name never reaches the network.
#[derive(Clone, Copy, Debug, Default)]
pub struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let looked = tokio::task::spawn_blocking(move || {
                use std::net::ToSocketAddrs as _;
                (host.as_str(), 0u16).to_socket_addrs().map(|addrs| {
                    let mut out = Vec::new();
                    for addr in addrs {
                        if public_address(addr.ip()) {
                            out.push(addr);
                        }
                    }
                    out
                })
            })
            .await;
            let error = |text: &str| -> Box<dyn std::error::Error + Send + Sync> { text.into() };
            let addrs = match looked {
                Ok(Ok(addrs)) => addrs,
                Ok(Err(_)) | Err(_) => return Err(error("the plugin host did not resolve")),
            };
            if addrs.is_empty() {
                return Err(error("the plugin host resolves to a non-public address"));
            }
            let addrs: Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
}

/// Whether `ip` is a public unicast address a plugin may talk to. Loopback,
/// link-local, private, carrier-grade NAT, multicast, unspecified, broadcast,
/// reserved and documentation ranges are refused.
pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_ipv4(v4),
        IpAddr::V6(v6) => public_ipv6(v6),
    }
}

fn public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 0.0.0.0/8 "this network", 100.64/10 carrier-grade NAT,
        // 198.18/15 benchmarking, 240/4 reserved.
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 198 && (18..=19).contains(&b))
        || a >= 240
        // 192.0.0.0/24 and 192.0.0.0/29 are not all documentation, but none
        // of them is a service a GIF provider uses.
        || (a == 192 && b == 0 && c == 0))
}

fn public_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_ipv4(v4);
    }
    let segments = ip.segments();
    // fc00::/7 unique local, fe80::/10 link-local.
    !((segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80)
}

/// The host backend of one plugin instance.
pub struct AppBackend {
    manifest: Arc<Manifest>,
    info: AppInfo,
    transport: Arc<dyn Transport>,
    secrets: Arc<dyn Secrets>,
    plugins: PluginsSettings,
}

impl AppBackend {
    pub fn new(
        manifest: Arc<Manifest>,
        transport: Arc<dyn Transport>,
        secrets: Arc<dyn Secrets>,
        plugins: PluginsSettings,
        info: AppInfo,
    ) -> Self {
        Self {
            manifest,
            info,
            transport,
            secrets,
            plugins,
        }
    }

    /// The manifest grant's hosts, checked again here before the transport
    /// sees anything.
    fn host_allowed(&self, host: &str) -> bool {
        self.manifest
            .capabilities
            .network
            .iter()
            .any(|h| h.matches(host))
    }
}

impl HostBackend for AppBackend {
    fn app_info(&self) -> AppInfo {
        self.info.clone()
    }

    fn http_fetch(
        &self,
        _plugin: &PluginId,
        request: HttpRequest,
        max_body: usize,
    ) -> BoxFuture<'_, Result<HttpResponse, ApiError>> {
        Box::pin(async move {
            let url = Url::parse(&request.url).map_err(|_| ApiError::Invalid("url".to_owned()))?;
            if url.scheme() != "https" {
                return Err(ApiError::Invalid("https".to_owned()));
            }
            let host = match url.host() {
                Some(Host::Domain(host)) => host.to_owned(),
                _ => return Err(ApiError::Denied("network".to_owned())),
            };
            if !self.host_allowed(&host) {
                return Err(ApiError::Denied(format!("network: {host}")));
            }
            let response = self
                .transport
                .fetch(self.manifest.id.as_str(), request, max_body)
                .await?;
            if response.body.len() > max_body {
                return Err(ApiError::Limit("response body".to_owned()));
            }
            Ok(response)
        })
    }

    fn secret_get(
        &self,
        plugin: &PluginId,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<String>, ApiError>> {
        let secrets = self.secrets.clone();
        let (plugin, key) = (plugin.as_str().to_owned(), key.to_owned());
        Box::pin(async move {
            let seen = secrets.get(&plugin, &key).await.map_err(ApiError::Failed)?;
            Ok(seen.map(|value| value.expose().to_owned()))
        })
    }

    fn secret_set(
        &self,
        plugin: &PluginId,
        key: &str,
        value: &str,
    ) -> BoxFuture<'_, Result<(), ApiError>> {
        let secrets = self.secrets.clone();
        let (plugin, key) = (plugin.as_str().to_owned(), key.to_owned());
        let value = SecretText::new(value);
        Box::pin(async move {
            secrets
                .set(&plugin, &key, value)
                .await
                .map_err(ApiError::Failed)
        })
    }

    fn secret_delete(&self, plugin: &PluginId, key: &str) -> BoxFuture<'_, Result<(), ApiError>> {
        let secrets = self.secrets.clone();
        let (plugin, key) = (plugin.as_str().to_owned(), key.to_owned());
        Box::pin(async move {
            secrets
                .delete(&plugin, &key)
                .await
                .map_err(ApiError::Failed)
        })
    }

    fn secret_keys(&self, plugin: &PluginId) -> BoxFuture<'_, Result<Vec<String>, ApiError>> {
        let secrets = self.secrets.clone();
        let plugin = plugin.as_str().to_owned();
        Box::pin(async move { secrets.keys(&plugin).await.map_err(ApiError::Failed) })
    }

    fn setting(
        &self,
        plugin: &PluginId,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<SettingValue>, ApiError>> {
        let value = self
            .plugins
            .entry(plugin.as_str())
            .and_then(|entry| entry.config.get(key))
            .and_then(setting_value);
        Box::pin(async move { Ok(value) })
    }
}

/// One `plugins.<id>.config` JSON value as the host's variant.
fn setting_value(value: &serde_json::Value) -> Option<SettingValue> {
    match value {
        serde_json::Value::Bool(on) => Some(SettingValue::Toggle(*on)),
        serde_json::Value::Number(n) => n.as_f64().map(SettingValue::Number),
        serde_json::Value::String(text) => Some(SettingValue::Text(text.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr as _;

    fn ip(s: &str) -> IpAddr {
        IpAddr::from_str(s).unwrap()
    }

    #[test]
    fn loopback_link_local_and_private_addresses_are_refused() {
        for text in [
            "127.0.0.1",
            "127.10.0.9",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "fc00::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
        ] {
            assert!(!public_address(ip(text)), "{text}");
        }
    }

    #[test]
    fn public_addresses_pass() {
        for text in [
            "8.8.8.8",
            "1.1.1.1",
            "140.82.121.3",
            "172.15.0.1",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(public_address(ip(text)), "{text}");
        }
    }
}
