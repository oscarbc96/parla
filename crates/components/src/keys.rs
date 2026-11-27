//! Key bindings whose keystroke string is built at run time.

/// Builds a binding from a keystroke string that only exists at run time.
/// Static keystrokes use `gpui::KeyBinding::new` instead.
pub(crate) fn binding<A: gpui::Action>(
    keys: &str,
    action: A,
    context: Option<&str>,
    cx: &gpui::App,
) -> gpui::KeyBinding {
    gpui::KeyBinding::load(
        keys,
        Box::new(action),
        context.map(|c| gpui::KeyBindingContextPredicate::parse(c).unwrap().into()),
        false,
        None,
        cx,
    )
    .unwrap()
}
