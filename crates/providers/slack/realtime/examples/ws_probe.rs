//! Manual spike probe for the session websocket.
//!
//! Usage: `cargo run -p slack_realtime --example ws_probe -- <rtm|client>
//! [--seconds N] [--agent-version TS] [--presence]`
//!
//! stdin, three lines (never argv or env): workspace URL (`https://<name>.slack.com`),
//! session token (`xoxc-...`), `d` cookie value (`xoxd-...`, as the browser stores it).
//!
//! Prints only: step results, handshake response header NAMES (plus the value of
//! `Sec-WebSocket-Extensions`), event `type` names with counts, close codes and
//! control-ping gaps. Never a payload, URL, token, cookie or ID. Any error text is
//! scrubbed of the token and cookie before printing.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use url::Url;

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const ORIGIN: &str = "https://app.slack.com";
/// `agent_version` seen in the 2026-11-13 captures (the web client's build timestamp).
const DEFAULT_AGENT_VERSION: &str = "1794521867";
const SILENCE_BEFORE_PING: Duration = Duration::from_secs(60);

struct Secrets {
    token: String,
    cookie: String,
}

impl Secrets {
    fn scrub(&self, text: &str) -> String {
        text.replace(&self.token, "<token>")
            .replace(&self.cookie, "<cookie>")
    }
}

fn read_stdin() -> (String, Secrets) {
    let mut lines = std::io::stdin().lock().lines();
    let mut next = |what: &str| {
        lines
            .next()
            .and_then(Result::ok)
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| {
                eprintln!("stdin line missing: {what}");
                std::process::exit(2)
            })
    };
    let workspace = next("workspace URL");
    let token = next("token");
    let cookie = next("cookie");
    (workspace, Secrets { token, cookie })
}

/// Event type names are printed, so keep only a short, plain identifier.
fn clean(name: &str) -> String {
    let s: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .take(40)
        .collect();
    if s.is_empty() { "(unnamed)".into() } else { s }
}

async fn api(
    http: &reqwest::Client,
    workspace: &str,
    method: &str,
    secrets: &Secrets,
) -> Result<serde_json::Value, String> {
    let resp = http
        .post(format!("{}/api/{method}", workspace.trim_end_matches('/')))
        .header("Cookie", format!("d={}", secrets.cookie))
        .header("Origin", ORIGIN)
        .form(&[("token", secrets.token.as_str())])
        .send()
        .await
        .map_err(|e| secrets.scrub(&e.without_url().to_string()))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| secrets.scrub(&e.without_url().to_string()))?;
    println!("{method}: http {status}, ok={}", body["ok"]);
    if body["ok"] != true {
        println!(
            "{method}: error code {}",
            clean(body["error"].as_str().unwrap_or("?"))
        );
    }
    Ok(body)
}

/// The URL the web client builds (2026-11-13 capture). `agent_version` equals the
/// `_x_version_ts` of its `/api` calls.
fn client_url(token: &str, team_id: &str, agent_version: &str) -> Url {
    let start_args = format!(
        "?agent=client&org_wide_aware=true&agent_version={agent_version}&eac_cache_ts=true\
&cache_ts=0&name_tagging=true&only_self_subteams=true&connect_only=true&ms_latest=true"
    );
    let mut url = Url::parse("wss://wss-primary.slack.com/").expect("static url");
    url.query_pairs_mut()
        .append_pair("token", token)
        .append_pair("sync_desync", "1")
        .append_pair("slack_client", "desktop")
        .append_pair("start_args", &start_args)
        .append_pair("no_query_on_subscribe", "1")
        .append_pair("flannel", "3")
        .append_pair("lazy_channels", "1")
        .append_pair("gateway_server", &format!("{team_id}-1"))
        .append_pair("batch_presence_aware", "1");
    url
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let mut seconds = 300u64;
    let mut agent_version = DEFAULT_AGENT_VERSION.to_string();
    let mut presence = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().and_then(|v| v.parse().ok()).unwrap_or(300),
            "--agent-version" => agent_version = args.next().unwrap_or(agent_version),
            "--presence" => presence = true,
            _ => {}
        }
    }
    if mode != "rtm" && mode != "client" {
        eprintln!("usage: ws_probe <rtm|client> [--seconds N] [--agent-version TS] [--presence]");
        std::process::exit(2);
    }
    let (workspace, secrets) = read_stdin();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(USER_AGENT)
        .build()
        .expect("http client");

    let auth = match api(&http, &workspace, "auth.test", &secrets).await {
        Ok(v) if v["ok"] == true => v,
        Ok(_) => std::process::exit(1),
        Err(e) => {
            println!("auth.test failed: {e}");
            std::process::exit(1)
        }
    };
    let team_id = auth["team_id"].as_str().unwrap_or_default().to_string();
    let self_user = auth["user_id"].as_str().unwrap_or_default().to_string();

    let url = if mode == "rtm" {
        match api(&http, &workspace, "rtm.connect", &secrets).await {
            Ok(v) if v["ok"] == true => match v["url"].as_str().and_then(|u| Url::parse(u).ok()) {
                Some(u) => {
                    let mut names: Vec<String> = u.query_pairs().map(|(k, _)| clean(&k)).collect();
                    names.sort();
                    println!(
                        "rtm.connect url: host-class ok, query params: {}",
                        names.join(",")
                    );
                    u
                }
                None => {
                    println!("rtm.connect: no usable url");
                    std::process::exit(1)
                }
            },
            _ => std::process::exit(1),
        }
    } else {
        client_url(&secrets.token, &team_id, &agent_version)
    };

    let mut request = url.as_str().into_client_request().expect("ws request");
    let h = request.headers_mut();
    h.insert(
        "Cookie",
        HeaderValue::from_str(&format!("d={}", secrets.cookie)).expect("cookie"),
    );
    h.insert("Origin", HeaderValue::from_static(ORIGIN));
    h.insert("User-Agent", HeaderValue::from_static(USER_AGENT));

    let (mut ws, response) = match tokio_tungstenite::connect_async(request).await {
        Ok(ok) => ok,
        Err(WsError::Http(resp)) => {
            println!("handshake rejected: http {}", resp.status());
            std::process::exit(1)
        }
        Err(e) => {
            println!("connect failed: {}", secrets.scrub(&e.to_string()));
            std::process::exit(1)
        }
    };
    println!("handshake: http {}", response.status());
    let mut names: Vec<String> = response
        .headers()
        .keys()
        .map(|k| k.as_str().to_string())
        .collect();
    names.sort();
    println!("handshake response headers: {}", names.join(","));
    println!(
        "sec-websocket-extensions: {}",
        response
            .headers()
            .get("sec-websocket-extensions")
            .and_then(|v| v.to_str().ok())
            .map(clean)
            .unwrap_or_else(|| "(none)".into())
    );

    let start = Instant::now();
    let deadline = start + Duration::from_secs(seconds);
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut next_id = 1u64;
    let mut last_inbound = Instant::now();
    let mut last_ctl_ping: Option<Instant> = None;
    let mut ctl_gaps: Vec<u64> = Vec::new();
    let mut close_note = String::from("(deadline reached, closed by probe)");

    if presence && !self_user.is_empty() {
        let frame = serde_json::json!({"id": next_id, "type": "presence_sub", "ids": [self_user]});
        next_id += 1;
        let _ = ws.send(Message::text(frame.to_string())).await;
        println!("sent presence_sub (own user only)");
    }

    loop {
        let ping_at = last_inbound + SILENCE_BEFORE_PING;
        let wake = ping_at.min(deadline);
        tokio::select! {
            () = tokio::time::sleep_until(wake.into()) => {
                if Instant::now() >= deadline { break; }
                let frame = serde_json::json!({"id": next_id, "type": "ping"});
                next_id += 1;
                let _ = ws.send(Message::text(frame.to_string())).await;
                println!("+{}s sent ping after 60s silence", start.elapsed().as_secs());
                last_inbound = Instant::now();
            }
            msg = ws.next() => {
                let Some(msg) = msg else { close_note = "(stream ended without close frame)".into(); break; };
                last_inbound = Instant::now();
                let t = start.elapsed().as_secs();
                match msg {
                    Ok(Message::Text(text)) => {
                        let name = serde_json::from_str::<serde_json::Value>(text.as_str())
                            .ok()
                            .map(|v| match v["type"].as_str() {
                                Some(t) => clean(t),
                                None if v.get("reply_to").is_some() => "(ack)".into(),
                                None => "(untyped)".into(),
                            })
                            .unwrap_or_else(|| "(non-json)".into());
                        println!("+{t}s {name}");
                        *counts.entry(name).or_default() += 1;
                    }
                    Ok(Message::Binary(_)) => {
                        println!("+{t}s (binary)");
                        *counts.entry("(binary)".into()).or_default() += 1;
                    }
                    Ok(Message::Ping(_)) => {
                        if let Some(prev) = last_ctl_ping { ctl_gaps.push(prev.elapsed().as_secs()); }
                        last_ctl_ping = Some(Instant::now());
                        *counts.entry("(ws-ping)".into()).or_default() += 1;
                    }
                    Ok(Message::Pong(_)) => { *counts.entry("(ws-pong)".into()).or_default() += 1; }
                    Ok(Message::Close(frame)) => {
                        close_note = match frame {
                            Some(f) => format!("close frame code {}", u16::from(f.code)),
                            None => "close frame without code".into(),
                        };
                        break;
                    }
                    Ok(Message::Frame(_)) => {}
                    Err(e) => {
                        close_note = format!("error: {}", secrets.scrub(&e.to_string()));
                        break;
                    }
                }
            }
        }
    }
    let _ = ws.close(None).await;
    println!(
        "--- ended after {}s: {close_note}",
        start.elapsed().as_secs()
    );
    println!("--- event counts");
    for (name, n) in &counts {
        println!("{n:>5}  {name}");
    }
    if !ctl_gaps.is_empty() {
        println!("--- control-ping gaps (s): {ctl_gaps:?}");
    }
}
