//! Accessibility fallbacks for gpui-pre, which has no `aria_disabled`. `Window::debug_a11y_tree_json` needs an activated platform
//! adapter, and `TestAppContext` force-disables accessibility, so these tests
//! check the strings exposed to the accessibility tree instead.

mod common;

use components::a11y::{disabled_label, set_polite, unavailable};
use gpui::accesskit::{Live, Node, Role};

#[test]
fn disabled_control_says_unavailable_in_its_name() {
    let en = common::en();
    assert_eq!(
        disabled_label("Continue", false, &en).to_string(),
        "Continue"
    );
    assert_eq!(
        disabled_label("Continue", true, &en).to_string(),
        "Continue, unavailable"
    );

    let es = common::localizer("es");
    assert_eq!(
        disabled_label("Continuar", true, &es).to_string(),
        "Continuar, no disponible"
    );
    let ca = common::localizer("ca");
    assert_eq!(
        disabled_label("Continuar", true, &ca).to_string(),
        "Continuar, no disponible"
    );
}

#[test]
fn unnamed_disabled_control_has_a_localized_description() {
    assert_eq!(unavailable(&common::en()).to_string(), "Unavailable");
    assert_eq!(
        unavailable(&common::localizer("es")).to_string(),
        "No disponible"
    );
    assert_eq!(
        unavailable(&common::localizer("ca")).to_string(),
        "No disponible"
    );
}

#[test]
fn a_live_region_is_polite() {
    let mut status = Node::new(Role::Status);
    // AccessKit leaves `live` off until it is set.
    assert_ne!(status.live(), Some(Live::Polite));
    set_polite(&mut status);
    assert_eq!(status.live(), Some(Live::Polite));
}
