//! Verified update downloads, without UI or installation.

use std::error::Error as _;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::redirect::Policy;

use super::package;

pub const MAX_UPDATE_BYTES: u64 = 300 * 1024 * 1024;
pub const MAX_SIGNATURE_BYTES: usize = 4096;

const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    Busy,
    TooLarge,
    SizeMismatch,
    Http,
    Io,
    Verify(package::VerifyError),
    Insecure,
}

pub struct Downloader {
    http: reqwest::Client,
    dir: PathBuf,
    busy: AtomicBool,
    allow_http: bool,
}

#[derive(Debug)]
struct InsecureRedirect;

impl fmt::Display for InsecureRedirect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for InsecureRedirect {}

struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct PartGuard {
    path: PathBuf,
    keep: bool,
}

impl Drop for PartGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn client(current: &semver::Version, allow_http: bool) -> reqwest::Client {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/octet-stream"));
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(&format!("Parla/{current}")).expect("version user agent"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .redirect(Policy::custom(move |attempt| {
            if !allow_http && attempt.url().scheme() != "https" {
                attempt.error(InsecureRedirect)
            } else if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .expect("static update client configuration is valid")
}

fn map_http(error: reqwest::Error) -> DownloadError {
    let mut source = error.source();
    while let Some(cause) = source {
        if cause.downcast_ref::<InsecureRedirect>().is_some() {
            return DownloadError::Insecure;
        }
        source = cause.source();
    }
    DownloadError::Http
}

async fn get(client: &reqwest::Client, url: &url::Url) -> Result<reqwest::Response, DownloadError> {
    let response = client.get(url.clone()).send().await.map_err(map_http)?;
    if response.status().is_redirection() {
        return Err(DownloadError::Http);
    }
    Ok(response)
}

fn check_url(url: &url::Url, allow_http: bool) -> Result<(), DownloadError> {
    if url.scheme() == "https" || (allow_http && url.scheme() == "http") {
        Ok(())
    } else {
        Err(DownloadError::Insecure)
    }
}

fn clean_dir(dir: &Path) -> Result<(), DownloadError> {
    std::fs::create_dir_all(dir).map_err(|_| DownloadError::Io)?;
    for entry in std::fs::read_dir(dir).map_err(|_| DownloadError::Io)? {
        let entry = entry.map_err(|_| DownloadError::Io)?;
        if !entry.file_type().map_err(|_| DownloadError::Io)?.is_dir() {
            std::fs::remove_file(entry.path()).map_err(|_| DownloadError::Io)?;
        }
    }
    Ok(())
}

impl Downloader {
    /// Production: `dir` is `settings::paths::cache_dir().join("updates")`.
    /// Only https URLs and https redirects are allowed.
    pub fn new(dir: PathBuf, current: &semver::Version) -> Self {
        Self::with_scheme_policy(dir, current, false)
    }

    /// Tests only: permits http URLs and redirects for a local mock server.
    pub fn new_allow_http(dir: PathBuf, current: &semver::Version) -> Self {
        Self::with_scheme_policy(dir, current, true)
    }

    fn with_scheme_policy(dir: PathBuf, current: &semver::Version, allow_http: bool) -> Self {
        Self {
            http: client(current, allow_http),
            dir,
            busy: AtomicBool::new(false),
            allow_http,
        }
    }

    async fn signature(&self, url: &url::Url) -> Result<String, DownloadError> {
        let mut response = get(&self.http, url)
            .await?
            .error_for_status()
            .map_err(|_| DownloadError::Http)?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_SIGNATURE_BYTES as u64)
        {
            return Err(DownloadError::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(map_http)? {
            if chunk.len() > MAX_SIGNATURE_BYTES - bytes.len() {
                return Err(DownloadError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| DownloadError::Verify(package::VerifyError::Invalid))
    }

    /// Downloads the signature and dmg into `dir`, verifies them, and returns
    /// the final dmg path.
    pub async fn download(
        &self,
        pkg: &package::UpdatePackage,
        public_key: &str,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<PathBuf, DownloadError> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(DownloadError::Busy);
        }
        let _busy = BusyGuard(&self.busy);

        if pkg.dmg.size == 0 || pkg.dmg.size > MAX_UPDATE_BYTES {
            return Err(DownloadError::TooLarge);
        }
        check_url(&pkg.signature.url, self.allow_http)?;
        check_url(&pkg.dmg.url, self.allow_http)?;
        clean_dir(&self.dir)?;

        let signature = self.signature(&pkg.signature.url).await?;
        let mut response = get(&self.http, &pkg.dmg.url)
            .await?
            .error_for_status()
            .map_err(|_| DownloadError::Http)?;
        let part_path = self.dir.join(format!("{}.part", pkg.dmg.name));
        let final_path = self.dir.join(&pkg.dmg.name);
        let mut part = PartGuard {
            path: part_path.clone(),
            keep: false,
        };
        let mut file = std::fs::File::create(&part_path).map_err(|_| DownloadError::Io)?;
        let mut received = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(map_http)? {
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or(DownloadError::TooLarge)?;
            if received > pkg.dmg.size {
                return Err(DownloadError::TooLarge);
            }
            file.write_all(&chunk).map_err(|_| DownloadError::Io)?;
            progress(received, pkg.dmg.size);
        }
        if received != pkg.dmg.size {
            return Err(DownloadError::SizeMismatch);
        }
        package::verify_file(pkg, &part_path, &signature, public_key)
            .map_err(DownloadError::Verify)?;
        file.flush().map_err(|_| DownloadError::Io)?;
        file.sync_all().map_err(|_| DownloadError::Io)?;
        drop(file);
        std::fs::rename(&part_path, &final_path).map_err(|_| DownloadError::Io)?;
        part.keep = true;
        Ok(final_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn https_only_client_refuses_an_http_redirect() {
        let server = MockServer::start().await;
        let target = format!("{}/final", server.uri());
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", target))
            .mount(&server)
            .await;
        let client = client(&semver::Version::new(0, 5, 0), false);
        let start = url::Url::parse(&format!("{}/start", server.uri())).unwrap();

        let result = get(&client, &start).await;

        assert!(matches!(result, Err(DownloadError::Insecure)));
    }
}
