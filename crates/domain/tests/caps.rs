//! The provider feature flags plugin areas map to:
//! files, read state, and separate message and file search.

use domain::test_support::{FakeProvider, uid};
use domain::{Capabilities, ChatProvider};

#[test]
fn plugin_area_flags_default_off() {
    let c = Capabilities::default();
    assert!(!c.files);
    assert!(!c.read_state);
    assert!(!c.search_messages);
    assert!(!c.search_files);
}

#[test]
fn fake_provider_reports_plugin_area_flags() {
    let p = FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
        files: true,
        read_state: true,
        search_messages: true,
        search_files: true,
        ..Capabilities::default()
    });
    let c = p.capabilities();
    assert!(c.files && c.read_state && c.search_messages && c.search_files);
    assert!(!FakeProvider::new("w2", uid("u1")).capabilities().files);
}
