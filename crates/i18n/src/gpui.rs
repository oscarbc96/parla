//! The app's localizer as a GPUI global: initialised before the first window,
//! switched live when the `locale` setting or the OS languages change.

use std::sync::Arc;

use gpui::{App, Global};

use crate::{Catalog, LocalePref, LocaleSet, Localizer, Translate, chain};

type Hook = Box<dyn Fn(&mut App)>;

/// The active localizer plus what is needed to rebuild it.
pub struct I18n {
    localizer: Arc<Localizer>,
    catalogs: Vec<&'static Catalog>,
    pref: LocalePref,
    hooks: Vec<Hook>,
}

impl Global for I18n {}

impl Translate for I18n {
    fn localizer(&self) -> &Localizer {
        &self.localizer
    }
}

impl Translate for App {
    fn localizer(&self) -> &Localizer {
        self.global::<I18n>().localizer.as_ref()
    }
}

/// The OS language list, most preferred first.
pub fn os_languages() -> Vec<String> {
    sys_locale::get_locales().collect()
}

fn build(catalogs: &[&'static Catalog], pref: &LocalePref, os: &[String]) -> Option<Localizer> {
    let chain = chain(pref, os, LocaleSet::embedded());
    let refs: Vec<&Catalog> = catalogs.to_vec();
    Localizer::new(chain, &refs).ok()
}

/// Builds for `pref`, falling back to the source locale when that fails.
fn build_or_source(
    catalogs: &[&'static Catalog],
    pref: &LocalePref,
    os: &[String],
) -> Option<Localizer> {
    build(catalogs, pref, os).or_else(|| {
        build(
            catalogs,
            &LocalePref::Code(LocaleSet::embedded().source.clone()),
            os,
        )
    })
}

/// Installs the global with the given core catalogs. The `i18n` crate's own
/// catalog is always included. Call before the first window opens.
pub fn init(cx: &mut App, catalogs: &[&'static Catalog], pref: LocalePref) {
    let os = os_languages();
    let localizer = build_or_source(catalogs, &pref, &os).expect("the source catalogs build");
    cx.set_global(I18n {
        localizer: Arc::new(localizer),
        catalogs: catalogs.to_vec(),
        pref,
        hooks: Vec::new(),
    });
}

/// Adds a catalog (a provider's) after [`init`] and rebuilds the localizer for
/// the current preference. A namespace already registered is left alone.
pub fn add_catalog(cx: &mut App, catalog: &'static Catalog) {
    let state = cx.global::<I18n>();
    if state
        .catalogs
        .iter()
        .any(|c| c.namespace() == catalog.namespace())
    {
        return;
    }
    let mut catalogs = state.catalogs.clone();
    catalogs.push(catalog);
    let os = os_languages();
    let rebuilt = build_or_source(&catalogs, &state.pref, &os);
    // A catalog that does not build (a missing source) is skipped, not fatal.
    if let Some(localizer) = rebuilt {
        let state = cx.global_mut::<I18n>();
        state.localizer = Arc::new(localizer);
        state.catalogs = catalogs;
    }
}

/// The active localizer; cheap to clone.
pub fn localizer(cx: &App) -> Arc<Localizer> {
    cx.global::<I18n>().localizer.clone()
}

/// Registers a hook run after every language switch (not on a same-chain reload).
pub fn on_relocalize(cx: &mut App, hook: impl Fn(&mut App) + 'static) {
    cx.global_mut::<I18n>().hooks.push(Box::new(hook));
}

/// Applies a new preference. When the fallback chain changes, rebuilds the
/// localizer, runs the hooks and refreshes every window; returns whether it did.
pub fn set_pref(cx: &mut App, pref: LocalePref) -> bool {
    set_pref_with_os(cx, pref, &os_languages())
}

/// [`set_pref`] with an injected OS language list.
pub fn set_pref_with_os(cx: &mut App, pref: LocalePref, os: &[String]) -> bool {
    let state = cx.global::<I18n>();
    let next_chain = chain(&pref, os, LocaleSet::embedded());
    if next_chain == state.localizer.chain() {
        cx.global_mut::<I18n>().pref = pref;
        return false;
    }
    let Some(localizer) = build(&state.catalogs, &pref, os) else {
        return false;
    };
    let state = cx.global_mut::<I18n>();
    state.localizer = Arc::new(localizer);
    state.pref = pref;
    let hooks = std::mem::take(&mut state.hooks);
    for hook in &hooks {
        hook(cx);
    }
    // Hooks registered while running are kept after the existing ones.
    let state = cx.global_mut::<I18n>();
    let added = std::mem::replace(&mut state.hooks, hooks);
    state.hooks.extend(added);
    cx.refresh_windows();
    true
}
