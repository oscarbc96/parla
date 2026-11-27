//! Shared harness for the tool tests: an rmcp client over `tokio::io::duplex`
//! in both client lifecycles against a `ParlaServer` on a `FakeBackend`.
#![allow(dead_code)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use domain::{
    Conversation, Message, PersistOp,
    test_support::{FakeProvider, conv, member, mid, uid},
};
use mcp::{
    audit::Auditor,
    limits::Limits,
    policy::SettingsSource,
    server::{Confirmer, DenyAllConfirmer, ParlaServer},
    test_support::FakeBackend,
};
use rmcp::{RoleClient, ServiceExt, model::CallToolRequestParams, service::RunningService};
use serde_json::{Value, json};
use settings::{McpSettings, McpWorkspaceSettings, WorkspaceKey};
use store::{MemoryAppStore, Store};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

/// MCP settings and the `log.include_content` switch.
pub struct Settings(pub Mutex<McpSettings>, pub AtomicBool);

impl SettingsSource for Settings {
    fn mcp(&self) -> Arc<McpSettings> {
        Arc::new(self.0.lock().unwrap().clone())
    }
    fn include_content(&self) -> bool {
        self.1.load(Ordering::Relaxed)
    }
}

pub fn ws(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

pub fn ts(n: u64) -> String {
    format!("{}.000000", 1_700_000_000 + n)
}

pub fn text_msg(c: &str, n: u64, text: &str) -> Message {
    Message::new(mid(c, &ts(n)), Some(uid("u2")), Arc::from(text))
}

#[derive(Clone, Copy, Debug)]
pub enum Lifecycle {
    Initialize,
    Inline,
}

pub const LIFECYCLES: [Lifecycle; 2] = [Lifecycle::Initialize, Lifecycle::Inline];

pub enum Client {
    Init(RunningService<RoleClient, ()>),
    Raw {
        w: WriteHalf<DuplexStream>,
        r: tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>,
        id: u64,
    },
}

impl Client {
    /// The JSON-RPC result, or the error object, of one request.
    pub async fn rpc_result(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let Client::Raw { w, r, id } = self else {
            unreachable!()
        };
        *id += 1;
        let mut params = params;
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "inline-client", "version": "9.9"},
            "io.modelcontextprotocol/clientCapabilities": {},
        });
        let line = json!({"jsonrpc": "2.0", "id": *id, "method": method, "params": params});
        w.write_all(format!("{line}\n").as_bytes()).await.unwrap();
        loop {
            let l = r.next_line().await.unwrap().expect("server closed");
            let v: Value = serde_json::from_str(&l).unwrap();
            if v["id"] == json!(*id) {
                return match v.get("error") {
                    Some(e) => Err(e.clone()),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    }

    pub async fn rpc(&mut self, method: &str, params: Value) -> Value {
        match self.rpc_result(method, params).await {
            Ok(v) => v,
            Err(e) => panic!("protocol error: {e}"),
        }
    }

    /// `resources/list` items.
    pub async fn resources(&mut self) -> Value {
        match self {
            Client::Init(c) => serde_json::to_value(c.list_all_resources().await.unwrap()).unwrap(),
            Client::Raw { .. } => self.rpc("resources/list", json!({})).await["resources"].clone(),
        }
    }

    /// `resources/templates/list` items.
    pub async fn templates(&mut self) -> Value {
        match self {
            Client::Init(c) => {
                serde_json::to_value(c.list_all_resource_templates().await.unwrap()).unwrap()
            }
            Client::Raw { .. } => {
                self.rpc("resources/templates/list", json!({})).await["resourceTemplates"].clone()
            }
        }
    }

    /// `resources/read`: the text of the first content block, or the error text.
    pub async fn read_resource(&mut self, uri: &str) -> Result<String, String> {
        match self {
            Client::Init(c) => c
                .read_resource(rmcp::model::ReadResourceRequestParams::new(uri))
                .await
                .map(|r| {
                    serde_json::to_value(&r).unwrap()["contents"][0]["text"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .map_err(|e| e.to_string()),
            Client::Raw { .. } => self
                .rpc_result("resources/read", json!({"uri": uri}))
                .await
                .map(|v| v["contents"][0]["text"].as_str().unwrap().to_string())
                .map_err(|e| e["message"].as_str().unwrap_or_default().to_string()),
        }
    }

    /// The tool result as JSON (`content`, `structuredContent`, `isError`, `_meta`).
    pub async fn call(&mut self, tool: &str, args: Value) -> Value {
        match self {
            Client::Init(c) => {
                let r = c
                    .call_tool(
                        CallToolRequestParams::new(tool.to_string())
                            .with_arguments(args.as_object().unwrap().clone()),
                    )
                    .await
                    .unwrap();
                serde_json::to_value(r).unwrap()
            }
            Client::Raw { .. } => {
                self.rpc("tools/call", json!({"name": tool, "arguments": args}))
                    .await
            }
        }
    }

    pub async fn tools(&mut self) -> Value {
        match self {
            Client::Init(c) => serde_json::to_value(c.list_all_tools().await.unwrap()).unwrap(),
            Client::Raw { .. } => self.rpc("tools/list", json!({})).await["tools"].clone(),
        }
    }
}

pub struct World {
    pub backend: Arc<FakeBackend>,
    pub providers: Vec<Arc<FakeProvider>>,
    pub settings: Arc<Settings>,
    pub limits: Arc<Limits>,
    pub audit_store: Arc<MemoryAppStore>,
    pub auditor: Auditor,
    pub client: Client,
    idempotency: Arc<mcp::idempotency::Idempotency>,
    confirmer: Arc<dyn Confirmer>,
}

impl World {
    /// Workspaces W1 and W2, both with MCP enabled unless `only_w1`.
    pub async fn new(lc: Lifecycle, providers: Vec<Arc<FakeProvider>>, enabled: &[&str]) -> World {
        World::build(lc, providers, enabled, Arc::new(DenyAllConfirmer), None).await
    }

    /// Like [`World::new`] with a chosen confirmer and, optionally, a confirmation timeout.
    pub async fn build(
        lc: Lifecycle,
        providers: Vec<Arc<FakeProvider>>,
        enabled: &[&str],
        confirmer: Arc<dyn Confirmer>,
        confirm_timeout: Option<Duration>,
    ) -> World {
        let backend = Arc::new(FakeBackend::new(providers.clone()));
        let mut s = McpSettings {
            enabled: true,
            ..McpSettings::default()
        };
        for id in enabled {
            s.workspaces.insert(
                ws(id).to_string(),
                McpWorkspaceSettings {
                    enabled: true,
                    include_dms: true,
                    ..McpWorkspaceSettings::default()
                },
            );
        }
        let settings = Arc::new(Settings(Mutex::new(s), AtomicBool::new(false)));
        let limits = Arc::new(Limits::system());
        let audit_store = Arc::new(MemoryAppStore::default());
        let auditor = Auditor::new(audit_store.clone(), settings.clone(), || 1_000);
        let idempotency = Arc::new(mcp::idempotency::Idempotency::default());
        let server = ParlaServer::new(
            backend.clone(),
            settings.clone(),
            limits.clone(),
            auditor.clone(),
            confirmer.clone(),
            Some(7),
        )
        .with_idempotency(idempotency.clone());
        let server = match confirm_timeout {
            Some(t) => server.with_confirm_timeout(t),
            None => server,
        };
        let client = connect(lc, server).await;
        World {
            backend,
            providers,
            settings,
            limits,
            audit_store,
            auditor,
            client,
            idempotency,
            confirmer,
        }
    }

    /// A new connection to the same server state, as after a shim reconnect.
    pub async fn reconnect(&self, lc: Lifecycle) -> Client {
        let server = ParlaServer::new(
            self.backend.clone(),
            self.settings.clone(),
            self.limits.clone(),
            self.auditor.clone(),
            self.confirmer.clone(),
            Some(7),
        )
        .with_idempotency(self.idempotency.clone());
        connect(lc, server).await
    }

    /// Fills the local store like a warm app: conversations, members and
    /// messages of every provider, so reads need no provider request.
    pub async fn warm(&self) {
        for p in &self.providers {
            let key = domain::ChatProvider::workspace(&**p).clone();
            let boot = domain::ChatProvider::bootstrap(&**p).await.unwrap();
            self.backend
                .store()
                .apply(&key, PersistOp::Conversations(boot.conversations))
                .await
                .unwrap();
            self.backend
                .store()
                .apply(&key, PersistOp::Members(boot.members))
                .await
                .unwrap();
        }
    }
}

async fn connect(lc: Lifecycle, server: ParlaServer) -> Client {
    let (a, b) = tokio::io::duplex(1 << 20);
    tokio::spawn(server.serve(a));
    match lc {
        Lifecycle::Initialize => Client::Init(().serve(b).await.expect("initialize")),
        Lifecycle::Inline => {
            let (r, w) = tokio::io::split(b);
            Client::Raw {
                w,
                r: BufReader::new(r).lines(),
                id: 0,
            }
        }
    }
}

pub fn provider(id: &str) -> Arc<FakeProvider> {
    let p = FakeProvider::new(id, uid("u1"));
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Alice"));
    Arc::new(p)
}

pub fn unread(id: &str, name: &str, last_read: u64, latest: u64, mentions: u32) -> Conversation {
    let mut c = conv(id, name);
    c.unread = true;
    c.mentions = mentions;
    c.last_read = Some(domain::test_support::ts(&ts(last_read)));
    c.latest = Some(domain::test_support::ts(&ts(latest)));
    c
}

pub fn payload(r: &Value) -> &Value {
    assert_ne!(r["isError"], json!(true), "unexpected error: {r}");
    &r["structuredContent"]
}

pub fn error_of(r: &Value) -> &Value {
    assert_eq!(r["isError"], json!(true), "expected an error: {r}");
    &r["structuredContent"]["error"]
}

/// Objects with sorted keys, so the snapshot does not depend on whether
/// `serde_json` keeps insertion order in this build.
pub fn canon(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), canon(&o[k])))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(canon).collect()),
        other => other.clone(),
    }
}
