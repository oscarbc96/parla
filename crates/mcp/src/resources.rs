//! Resources: nothing is enumerated; two neutral URI
//! templates name a conversation and a thread, and a read returns the same
//! JSON as `read_channel` (first page) or `read_thread`, through the same
//! policy, limits and audit.

use rmcp::model::{ResourceContents, ResourceTemplate};
use serde_json::Value;

use crate::{
    audit::AuditSpan,
    errors::ToolFailure,
    server::Shared,
    tools::read::{ReadChannelArgs, ReadThreadArgs},
};

pub const CONVERSATION_TEMPLATE: &str =
    "parla://{provider}/{workspace}/conversation/{conversation}";
pub const THREAD_TEMPLATE: &str = "parla://{provider}/{workspace}/thread/{conversation}/{ts}";
pub const SCHEME: &str = "parla://";

/// The two resource templates, in a stable order.
pub fn templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new(CONVERSATION_TEMPLATE, "conversation")
            .with_description(
                "The newest messages of a conversation: the same JSON as the read_channel tool, first page.",
            )
            .with_mime_type("application/json"),
        ResourceTemplate::new(THREAD_TEMPLATE, "thread")
            .with_description(
                "A thread, root first: the same JSON as the read_thread tool, first page.",
            )
            .with_mime_type("application/json"),
    ]
}

/// A parsed resource URI. `workspace` is `<provider>/<id>`.
#[derive(Debug, PartialEq, Eq)]
pub enum ResourceRef {
    Conversation {
        workspace: String,
        conversation: String,
    },
    Thread {
        workspace: String,
        conversation: String,
        ts: String,
    },
}

pub fn parse_uri(uri: &str) -> Result<ResourceRef, ToolFailure> {
    let bad = || {
        ToolFailure::bad_argument(format!(
            "unknown resource uri: expected {CONVERSATION_TEMPLATE} or {THREAD_TEMPLATE}"
        ))
    };
    let rest = uri.strip_prefix(SCHEME).ok_or_else(bad)?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(bad());
    }
    match parts.as_slice() {
        [provider, id, "conversation", c] => Ok(ResourceRef::Conversation {
            workspace: format!("{provider}/{id}"),
            conversation: (*c).to_string(),
        }),
        [provider, id, "thread", c, ts] => Ok(ResourceRef::Thread {
            workspace: format!("{provider}/{id}"),
            conversation: (*c).to_string(),
            ts: (*ts).to_string(),
        }),
        _ => Err(bad()),
    }
}

impl Shared {
    /// The resource's contents: the JSON text the matching tool returns.
    pub(crate) async fn read_resource_contents(
        &self,
        span: &mut AuditSpan,
        uri: &str,
    ) -> Result<ResourceContents, ToolFailure> {
        let result = match parse_uri(uri)? {
            ResourceRef::Conversation {
                workspace,
                conversation,
            } => {
                self.read_channel(
                    span,
                    ReadChannelArgs {
                        workspace,
                        conversation,
                        cursor: None,
                        limit: None,
                    },
                )
                .await?
            }
            ResourceRef::Thread {
                workspace,
                conversation,
                ts,
            } => {
                self.read_thread(
                    span,
                    ReadThreadArgs {
                        workspace,
                        conversation,
                        ts,
                        cursor: None,
                        limit: None,
                    },
                )
                .await?
            }
        };
        let text = result
            .structured_content
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default();
        let mut meta = serde_json::Map::new();
        meta.insert("parla/untrusted_content".into(), Value::Bool(true));
        Ok(ResourceContents::TextResourceContents {
            uri: uri.to_string(),
            mime_type: Some("application/json".into()),
            text,
            meta: Some(rmcp::model::MetaObject(meta)),
        })
    }
}
