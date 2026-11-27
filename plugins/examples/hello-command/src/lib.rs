//! Example plugin: the `greet` command shows a toast. The smallest plugin the
//! SDK supports: `impl Plugin` plus `export!` with the one extension point
//! its manifest declares.

use parla_plugin_sdk::host::ui::{self, ToastKind};
use parla_plugin_sdk::types::Context;
use parla_plugin_sdk::{Error, Plugin, export};

pub struct Hello;

impl Plugin for Hello {
    fn run_command(id: String, _context: Context) -> Result<(), Error> {
        match id.as_str() {
            "greet" => ui::toast("Hello from a plugin", ToastKind::Success),
            _ => Err(Error::NotFound),
        }
    }
}

export!(Hello with [commands]);

#[cfg(test)]
mod tests {
    use super::*;
    use parla_plugin_sdk::testing::FakeHost;

    fn ctx() -> Context {
        Context {
            workspace: None,
            conversation: None,
            thread: None,
            message: None,
        }
    }

    #[test]
    fn greet_shows_a_toast() {
        let fake = FakeHost::from_manifest(include_str!("../plugin.toml"));
        Hello::run_command("greet".into(), ctx()).unwrap();
        assert_eq!(
            fake.toasts(),
            vec![("Hello from a plugin".to_string(), ToastKind::Success)]
        );
    }

    #[test]
    fn unknown_command_is_not_found() {
        let _fake = FakeHost::from_manifest(include_str!("../plugin.toml"));
        assert!(matches!(
            Hello::run_command("nope".into(), ctx()),
            Err(Error::NotFound)
        ));
    }
}
