//! Store encryption keys in the OS keychain through GPUI's credentials API.
//!
//! One item per workspace: URL `parla://store/<provider>/<id>`, username
//! `parla`, password the 64-hex-digit key. GPUI's `AsyncApp` is not `Send`, so
//! requests hop to the main thread over a channel and a foreground task answers
//! them; callers never block the main thread.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use gpui::App;
use store::StoreError;
use store_sqlite::{DbId, Key, KeySource};

const USER: &str = "parla";

/// Keychain entry URL for a database's store key: `parla://store/app` for
/// `app.db`, `parla://store/<provider>/<id>` for a workspace.
pub fn entry_url(db: &DbId) -> String {
    match db {
        DbId::App => "parla://store/app".to_string(),
        DbId::Workspace(ws) => format!("parla://store/{}/{}", ws.provider.0, ws.id.0),
    }
}

type Reply<T> = oneshot::Sender<Result<T, StoreError>>;

enum Request {
    Read(String, Reply<Option<Vec<u8>>>),
    Write(String, Vec<u8>, Reply<()>),
    Delete(String, Reply<()>),
}

fn unavailable() -> StoreError {
    StoreError::Other("keychain unavailable".to_string())
}

fn keychain(e: impl std::fmt::Display) -> StoreError {
    StoreError::Other(format!("keychain: {e}"))
}

pub struct GpuiKeySource {
    tx: mpsc::UnboundedSender<Request>,
}

impl GpuiKeySource {
    pub fn new(cx: &mut App) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded::<Request>();
        cx.spawn(async move |cx| {
            while let Some(req) = rx.next().await {
                match req {
                    Request::Read(url, reply) => {
                        let r = cx.update(|cx| cx.read_credentials(&url)).await;
                        let _ = reply.send(r.map(|o| o.map(|(_, b)| b)).map_err(keychain));
                    }
                    Request::Write(url, bytes, reply) => {
                        let r = cx
                            .update(|cx| cx.write_credentials(&url, USER, &bytes))
                            .await;
                        let _ = reply.send(r.map_err(keychain));
                    }
                    Request::Delete(url, reply) => {
                        let r = cx.update(|cx| cx.delete_credentials(&url)).await;
                        let _ = reply.send(r.map_err(keychain));
                    }
                }
            }
        })
        .detach();
        Arc::new(Self { tx })
    }

    async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Request) -> Result<T, StoreError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .unbounded_send(make(reply))
            .map_err(|_| unavailable())?;
        rx.await.map_err(|_| unavailable())?
    }

    async fn store_new(&self, url: String) -> Result<Key, StoreError> {
        let key = Key::random();
        let hex = key.to_hex();
        self.ask(|r| Request::Write(url, hex.as_bytes().to_vec(), r))
            .await?;
        Ok(key)
    }
}

impl KeySource for GpuiKeySource {
    fn get_or_create(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        let url = entry_url(ws);
        Box::pin(async move {
            match self.ask(|r| Request::Read(url.clone(), r)).await? {
                Some(bytes) => {
                    let text = zeroize::Zeroizing::new(String::from_utf8_lossy(&bytes).to_string());
                    // A malformed item is never overwritten silently: the
                    // store reports unreadable and the user decides.
                    Key::from_hex(text.trim()).ok_or(StoreError::Unreadable)
                }
                None => self.store_new(url).await,
            }
        })
    }

    fn replace(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        let url = entry_url(ws);
        Box::pin(self.store_new(url))
    }

    fn delete(&self, ws: &DbId) -> BoxFuture<'_, Result<(), StoreError>> {
        let url = entry_url(ws);
        Box::pin(async move { self.ask(|r| Request::Delete(url, r)).await })
    }
}
