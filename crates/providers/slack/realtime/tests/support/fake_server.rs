#![allow(clippy::result_large_err)] // tungstenite's callback signature fixes the error type
//! In-process fake websocket server: records handshakes and client frames, plays a
//! script per accepted connection. No network beyond loopback.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

#[derive(Debug, Clone)]
pub enum Step {
    Send(String),
    Sleep(Duration),
    Close(u16, &'static str),
    /// Drop the TCP connection without a close frame.
    Drop,
}

#[derive(Debug, Clone, Default)]
pub struct Script {
    pub steps: Vec<Step>,
    /// Answer client JSON pings with a pong.
    pub reply_pong: bool,
}

#[derive(Debug, Clone)]
pub struct Handshake {
    pub headers: HashMap<String, String>,
    pub query: String,
}

#[derive(Default)]
struct State {
    scripts: VecDeque<Script>,
    handshakes: Vec<Handshake>,
    received: Vec<String>,
}

pub struct FakeServer {
    pub addr: std::net::SocketAddr,
    state: Arc<Mutex<State>>,
    notify: Arc<tokio::sync::Notify>,
}

impl FakeServer {
    /// `scripts` are used for the 1st, 2nd, ... connection; later ones hold open.
    pub async fn start(scripts: Vec<Script>) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State {
            scripts: scripts.into(),
            ..State::default()
        }));
        let st = state.clone();
        let notify = Arc::new(tokio::sync::Notify::new());
        let nt = notify.clone();
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let st = st.clone();
                let nt = nt.clone();
                tokio::spawn(async move {
                    let script = st.lock().unwrap().scripts.pop_front().unwrap_or_default();
                    let st2 = st.clone();
                    let cb = move |req: &Request, resp: Response| {
                        let headers = req
                            .headers()
                            .iter()
                            .map(|(k, v)| {
                                (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())
                            })
                            .collect();
                        st2.lock().unwrap().handshakes.push(Handshake {
                            headers,
                            query: req.uri().query().unwrap_or("").to_string(),
                        });
                        Ok(resp)
                    };
                    let Ok(mut ws) = accept_hdr_async(tcp, cb).await else {
                        return;
                    };
                    for step in &script.steps {
                        match step {
                            Step::Send(t) => {
                                let _ = ws.send(Message::text(t.clone())).await;
                            }
                            Step::Sleep(d) => tokio::time::sleep(*d).await,
                            Step::Close(code, reason) => {
                                let _ = ws
                                    .send(Message::Close(Some(CloseFrame {
                                        code: CloseCode::from(*code),
                                        reason: (*reason).into(),
                                    })))
                                    .await;
                            }
                            Step::Drop => return,
                        }
                    }
                    while let Some(Ok(msg)) = ws.next().await {
                        if let Message::Text(t) = msg {
                            let t = t.to_string();
                            st.lock().unwrap().received.push(t.clone());
                            nt.notify_waiters();
                            if script.reply_pong
                                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&t)
                                && v["type"] == "ping"
                            {
                                let _ = ws
                                    .send(Message::text(
                                        serde_json::json!({"reply_to": v["id"], "type": "pong"})
                                            .to_string(),
                                    ))
                                    .await;
                            }
                        }
                    }
                });
            }
        });
        FakeServer {
            addr,
            state,
            notify,
        }
    }

    pub fn url(&self) -> url::Url {
        url::Url::parse(&format!("ws://{}/", self.addr)).unwrap()
    }
    /// Queues a script for a later connection (used when it needs the server's own URL).
    pub fn push_script(&self, script: Script) {
        self.state.lock().unwrap().scripts.push_back(script);
    }
    pub fn handshakes(&self) -> Vec<Handshake> {
        self.state.lock().unwrap().handshakes.clone()
    }
    /// Event-driven wait until at least `n` client frames arrived.
    pub async fn wait_received(&self, n: usize) -> Vec<String> {
        loop {
            let waiter = self.notify.notified();
            tokio::pin!(waiter);
            waiter.as_mut().enable();
            let got = self.received();
            if got.len() >= n {
                return got;
            }
            waiter.await;
        }
    }
    pub fn received(&self) -> Vec<String> {
        self.state.lock().unwrap().received.clone()
    }
}
