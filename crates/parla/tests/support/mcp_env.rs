//! Shared setup for the MCP lifecycle tests: the fake accounts of `support`,
//! the real IPC listener in a short temp dir, an rmcp client over the socket.
#![allow(dead_code)]

use gpui::TestAppContext;
use mcp::ipc::IpcStream;
use parla::instance::{Hooks, Instance, LaunchMode};
use parla::mcp_host::McpHost;
use parla::workspaces::{Workspaces, WorkspacesHandle};
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::Value;
use std::path::PathBuf;

use super::{Env, env, settle, workspaces};

pub struct Mcp {
    pub env: Env,
    pub w: gpui::Entity<Workspaces>,
    pub rt: tokio::runtime::Runtime,
    pub socket: PathBuf,
    _run: tempfile::TempDir,
}

/// Settings enabling MCP for `fake/w1` (DMs visible, writes allowed without
/// confirmation), with `extra` merged into the top-level `mcp` object.
pub fn settings_json(enabled: bool, extra: &str) -> String {
    format!(
        r#"{{ "mcp": {{ "enabled": {enabled}, {extra} "workspaces": {{ "fake/w1": {{ "enabled": true, "allow_writes": true, "confirm_writes": false, "include_dms": true }} }} }} }}"#
    )
}

/// Three fake accounts, the listener bound, the MCP host installed.
pub fn start(cx: &mut TestAppContext, settings: &str) -> Mcp {
    let e = env(cx, settings);
    start_with(cx, e)
}

pub fn start_with(cx: &mut TestAppContext, e: Env) -> Mcp {
    let w = workspaces(cx, &e);
    let run = tempfile::Builder::new()
        .prefix("t")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = run.path().join("run").join("mcp.sock");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.handle().clone();
    let (s, wh) = (socket.clone(), w.clone());
    cx.update(|cx| {
        cx.set_global(WorkspacesHandle(wh));
        let hooks = Hooks {
            show: std::rc::Rc::new(|_| {}),
            quit: std::rc::Rc::new(|_| {}),
        };
        Instance::install(cx, &handle, &s, LaunchMode::Window, hooks).unwrap();
        McpHost::install(cx, handle);
    });
    for p in &e.providers {
        p.push(domain::ProviderEvent::Connected { reconnect: false });
    }
    settle(cx);
    Mcp {
        env: e,
        w,
        rt,
        socket,
        _run: run,
    }
}

impl Mcp {
    /// Rewrites `settings.json` and reloads it the way the file watcher does.
    pub fn set_settings(&self, cx: &mut TestAppContext, json: &str) {
        std::fs::write(self.env.config.join("settings.json"), json).unwrap();
        cx.update(|cx| parla::handle_change(cx, settings::ChangedFile::Settings));
        settle(cx);
    }

    /// Connects (handshake `mcp`) on the runtime.
    pub async fn connect(&self) -> Result<IpcStream, mcp::ipc::IpcError> {
        let p = self.socket.clone();
        self.rt
            .spawn(async move { mcp::ipc::connect(&p, "mcp").await })
            .await
            .unwrap()
    }

    /// One tool call from a fresh client; the first text block parsed as JSON,
    /// or the error text under `{"error": ...}`.
    pub async fn call(&self, tool: &'static str, args: Value) -> Value {
        let p = self.socket.clone();
        self.rt
            .spawn(async move {
                let stream = mcp::ipc::connect(&p, "mcp").await.expect("connect");
                let client = ().serve(stream).await.expect("initialize");
                let mut params = CallToolRequestParams::new(tool);
                if let Value::Object(m) = args {
                    params = params.with_arguments(m);
                }
                let r = client.call_tool(params).await.expect("call_tool");
                let v = serde_json::to_value(&r).unwrap();
                let text = v["content"][0]["text"].as_str().unwrap_or("").to_string();
                let out = serde_json::from_str::<Value>(&text)
                    .unwrap_or_else(|_| serde_json::json!({ "text": text }));
                if r.is_error == Some(true) {
                    serde_json::json!({ "error": out })
                } else {
                    out
                }
            })
            .await
            .unwrap()
    }
}
