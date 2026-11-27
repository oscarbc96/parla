//! File info, external upload and downloads that only talk to file hosts.
//!
//! The upload flow follows the public docs (`files.getUploadURLExternal`, POST the
//! bytes, `files.completeUploadExternal`); the web client's own variant is not
//! captured yet (see docs/providers/slack/web-client/files.md, UNVERIFIED).

use std::path::Path;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, LOCATION, ORIGIN, USER_AGENT};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::client::transport;
use crate::types::File;
use crate::{ApiError, Priority, WebClient, method};

const MAX_REDIRECTS: usize = 3;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(900);

/// True for `https` URLs on a Slack file host: `files.slack.com`,
/// `slack-files.com`, a subdomain of `slack-edge.com` or of `slack.com`.
pub fn is_file_host(url: &Url) -> bool {
    if url.scheme() != "https" || !matches!(url.port(), None | Some(443)) {
        return false;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    host == "files.slack.com"
        || host == "slack-files.com"
        || host.ends_with(".slack-files.com")
        || host.ends_with(".slack-edge.com")
        || host.ends_with(".slack.com")
}

/// The hosts allowed to receive file requests. The default is the production list.
#[derive(Debug, Clone, Default)]
pub struct FileHosts {
    /// `host:port` authorities admitted over plain http (tests only).
    #[cfg(feature = "test-support")]
    extra: Vec<String>,
}

impl FileHosts {
    /// Admits a mock server's `host:port` (any scheme). Test-only.
    #[cfg(feature = "test-support")]
    pub fn with_test_authority(mut self, authority: &str) -> Self {
        self.extra.push(authority.to_string());
        self
    }

    pub fn allows(&self, url: &Url) -> bool {
        #[cfg(feature = "test-support")]
        if !is_file_host(url) {
            return match (url.host_str(), url.port_or_known_default()) {
                (Some(h), Some(p)) => self.extra.iter().any(|a| *a == format!("{h}:{p}")),
                _ => false,
            };
        }
        is_file_host(url)
    }
}

fn authority(url: &Url) -> (Option<String>, Option<u16>) {
    (
        url.host_str().map(str::to_ascii_lowercase),
        url.port_or_known_default(),
    )
}

#[derive(Deserialize)]
struct InfoResponse {
    file: File,
}

#[derive(Deserialize)]
struct UploadUrl {
    upload_url: String,
    file_id: String,
}

#[derive(Deserialize)]
struct Completed {
    #[serde(default)]
    files: Vec<File>,
}

impl WebClient {
    pub async fn files_info(&self, file: &str) -> Result<File, ApiError> {
        let r: InfoResponse = self
            .call(method::FILES_INFO, &[("file", file)], Priority::Visible)
            .await?;
        Ok(r.file)
    }

    /// GET with credentials, only on file hosts. Redirects are followed by hand
    /// (at most 3); each hop must be a file host and credentials are dropped on a
    /// hop to a different host.
    async fn get_file(&self, url: &str) -> Result<reqwest::Response, ApiError> {
        let mut url = Url::parse(url).map_err(|_| ApiError::ForeignHost)?;
        if !self.cfg.file_hosts.allows(&url) {
            return Err(ApiError::ForeignHost);
        }
        let origin = authority(&url);
        let mut creds = true;
        for _ in 0..=MAX_REDIRECTS {
            let mut req = self
                .http
                .get(url.clone())
                .timeout(DOWNLOAD_TIMEOUT)
                .header(USER_AGENT, &self.cfg.user_agent);
            if creds {
                req = req
                    .header(COOKIE, self.cookie.header_value())
                    .header(AUTHORIZATION, format!("Bearer {}", self.token.expose()))
                    .header(ORIGIN, "https://app.slack.com");
            }
            let resp = req.send().await.map_err(transport)?;
            let status = resp.status();
            if status.is_redirection() {
                let next = resp
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|l| url.join(l).ok())
                    .ok_or(ApiError::Http {
                        status: status.as_u16(),
                    })?;
                if !self.cfg.file_hosts.allows(&next) {
                    return Err(ApiError::ForeignHost);
                }
                if authority(&next) != origin {
                    creds = false;
                }
                url = next;
                continue;
            }
            if !status.is_success() {
                return Err(ApiError::Http {
                    status: status.as_u16(),
                });
            }
            return Ok(resp);
        }
        Err(ApiError::Transport("too many redirects".to_string()))
    }

    /// Whole body in memory, at most `max_bytes`.
    pub async fn fetch_private(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, ApiError> {
        let mut resp = self.get_file(url).await?;
        if resp.content_length().is_some_and(|n| n > max_bytes) {
            return Err(ApiError::TooLarge { max: max_bytes });
        }
        let mut out = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(transport)? {
            if out.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(ApiError::TooLarge { max: max_bytes });
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// Streams to `<dest>.part` and renames on success; returns the byte count.
    pub async fn download_to(&self, url: &str, dest: &Path) -> Result<u64, ApiError> {
        let mut resp = self.get_file(url).await?;
        let mut part = dest.as_os_str().to_owned();
        part.push(".part");
        let part = std::path::PathBuf::from(part);
        let io = |e: std::io::Error| ApiError::Io(e.kind().to_string());
        let result = async {
            let mut f = tokio::fs::File::create(&part).await.map_err(io)?;
            let mut n = 0u64;
            while let Some(chunk) = resp.chunk().await.map_err(transport)? {
                f.write_all(&chunk).await.map_err(io)?;
                n += chunk.len() as u64;
            }
            f.flush().await.map_err(io)?;
            drop(f);
            tokio::fs::rename(&part, dest).await.map_err(io)?;
            Ok(n)
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        result
    }

    /// `files.getUploadURLExternal` per file, POST the bytes, then one
    /// `files.completeUploadExternal` that shares them all in one message.
    pub async fn upload_files(
        &self,
        channel: &str,
        thread_ts: Option<&str>,
        files: &[(&Path, &str, Option<&str>)],
        initial_comment: Option<&str>,
    ) -> Result<Vec<File>, ApiError> {
        let mut ids = Vec::new();
        for (path, name, snippet_type) in files {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|e| ApiError::Io(e.kind().to_string()))?;
            let len = bytes.len().to_string();
            // `snippet_type` makes the file a code snippet of that syntax.
            let mut params = vec![("filename", *name), ("length", len.as_str())];
            if let Some(t) = snippet_type {
                params.push(("snippet_type", t));
            }
            let u: UploadUrl = self
                .call(
                    method::FILES_GET_UPLOAD_URL_EXTERNAL,
                    &params,
                    Priority::User,
                )
                .await?;
            let target = Url::parse(&u.upload_url).map_err(|_| ApiError::ForeignHost)?;
            if !self.cfg.file_hosts.allows(&target) {
                return Err(ApiError::ForeignHost);
            }
            // The upload URL carries its own signature; no credentials are sent.
            let resp = self
                .http
                .post(target)
                .timeout(DOWNLOAD_TIMEOUT)
                .header(USER_AGENT, &self.cfg.user_agent)
                .header(CONTENT_TYPE, "application/octet-stream")
                .body(bytes)
                .send()
                .await
                .map_err(transport)?;
            if !resp.status().is_success() {
                return Err(ApiError::Http {
                    status: resp.status().as_u16(),
                });
            }
            ids.push(serde_json::json!({ "id": u.file_id, "title": name }));
        }
        let files_json = serde_json::Value::Array(ids).to_string();
        let mut params = vec![("files", files_json.as_str()), ("channel_id", channel)];
        if let Some(t) = thread_ts {
            params.push(("thread_ts", t));
        }
        if let Some(c) = initial_comment {
            params.push(("initial_comment", c));
        }
        let done: Completed = self
            .call_write(
                method::FILES_COMPLETE_UPLOAD_EXTERNAL,
                &params,
                Priority::User,
            )
            .await?;
        Ok(done.files)
    }
}
