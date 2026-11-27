//! Fallbacks for the accessibility gaps in gpui-fast 0.1.19:
//! there is no `aria_disabled`, so a disabled control says "unavailable" in
//! its accessible name or description instead, and no `aria_live`, so a live
//! region sets AccessKit's `live` on its node through the synthetic-children
//! hook ([`polite_live_region`]).

use gpui::SharedString;
use i18n::Localizer;

/// The accessible name of a control, marked unavailable when `disabled`.
pub fn disabled_label(
    label: impl Into<SharedString>,
    disabled: bool,
    l: &Localizer,
) -> SharedString {
    let label = label.into();
    if !disabled {
        return label;
    }
    let suffix = t!(l, "a11y-unavailable-suffix");
    format!("{label}{suffix}").into()
}

/// Accessible description of a disabled control that has no label of its own.
pub fn unavailable(l: &Localizer) -> SharedString {
    t!(l, "a11y-unavailable").into_owned().into()
}

/// Marks `node` a polite live region: assistive technology announces its
/// changes when idle. AccessKit leaves `live` off, and `Role::Status` alone
/// does not set it.
pub fn set_polite(node: &mut gpui::accesskit::Node) {
    node.set_live(gpui::accesskit::Live::Polite);
}

/// For `.a11y_synthetic_children(a11y::polite_live_region)` on an element
/// that has an id and a role (gpui has no `aria_live`): the element's own node
/// becomes a polite live region.
pub fn polite_live_region(builder: &mut gpui::A11ySubtreeBuilder) {
    set_polite(builder.parent_node());
}
