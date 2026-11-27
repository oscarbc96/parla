//! Slack web-client recon CLI: wires the provider-neutral `recon` tool with
//! Slack's redaction rules and read-only method list.

use std::process::ExitCode;
use std::sync::Arc;

use recon::cli::{Provider, run};

/// Methods the drift check may ever call. Everything else is refused.
const READ_ONLY_METHODS: &[&str] = &[
    "auth.test",
    "client.boot",
    "client.userBoot",
    "client.counts",
    "conversations.history",
    "conversations.replies",
    "conversations.info",
    "conversations.list",
    "users.conversations",
    "users.info",
    "users.list",
    "users.getPresence",
    "team.info",
    "emoji.list",
    "files.info",
];

fn main() -> ExitCode {
    run(&Provider {
        id: "slack".into(),
        rules: Arc::new(slack_redact::Rules::default()),
        read_only_methods: READ_ONLY_METHODS.iter().map(|m| (*m).to_string()).collect(),
    })
}
