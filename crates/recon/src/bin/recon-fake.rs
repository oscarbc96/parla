//! CLI wired with the made-up fake provider, for the tool's own tests.

use std::process::ExitCode;
use std::sync::Arc;

use recon::cli::{Provider, run};
use redact::FakeRules;

fn main() -> ExitCode {
    run(&Provider {
        id: "fake".into(),
        rules: Arc::new(FakeRules::default()),
        read_only_methods: vec!["items.list".into(), "items.info".into()],
    })
}
