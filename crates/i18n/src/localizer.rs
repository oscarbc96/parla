use std::borrow::Cow;
use std::collections::HashMap;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

use crate::catalog::{Catalog, CatalogError};
use crate::locales::PSEUDO;
use crate::text::LocalizedText;

type Bundle = FluentBundle<FluentResource>;

/// Formats messages for the active fallback chain: per namespace, one
/// bundle per chain entry, tried in order.
pub struct Localizer {
    chain: Vec<LanguageIdentifier>,
    /// The active locale parsed once for the ICU formatters.
    pub(crate) icu: icu_locale::Locale,
    bundles: HashMap<&'static str, Vec<Bundle>>,
}

impl Localizer {
    /// `chain` ends in the source locale. `en-XA` builds its bundle from the
    /// source catalog and pseudo-transforms it.
    pub fn new(
        chain: Vec<LanguageIdentifier>,
        catalogs: &[&Catalog],
    ) -> Result<Self, CatalogError> {
        let source = chain.last().ok_or(CatalogError::EmptyChain)?.clone();
        let mut bundles = HashMap::new();
        let own = &crate::CATALOG;
        let all = catalogs
            .iter()
            .copied()
            .chain((!catalogs.iter().any(|c| c.namespace() == own.namespace())).then_some(own));
        for cat in all {
            let mut per_chain = Vec::with_capacity(chain.len());
            for lang in &chain {
                let is_pseudo = *lang == PSEUDO;
                let file = if is_pseudo { &source } else { lang };
                let Some(src) = cat.source(&file.to_string()) else {
                    if *file == source {
                        return Err(CatalogError::MissingSource {
                            ns: cat.namespace().into(),
                            code: file.to_string(),
                        });
                    }
                    continue;
                };
                let parse_err = |message: String| CatalogError::Parse {
                    ns: cat.namespace().into(),
                    code: file.to_string(),
                    message,
                };
                let res = FluentResource::try_new(src.into_owned())
                    .map_err(|(_, errs)| parse_err(format!("{errs:?}")))?;
                let mut bundle = FluentBundle::new_concurrent(vec![lang.clone()]);
                bundle.set_use_isolating(false);
                // Registers NUMBER(); without it every NUMBER() placeable fails.
                bundle
                    .add_builtins()
                    .map_err(|e| parse_err(format!("{e:?}")))?;
                bundle.set_formatter(Some(crate::format::fluent_number));
                if is_pseudo {
                    bundle.set_transform(Some(pseudo));
                }
                bundle
                    .add_resource(res)
                    .map_err(|errs| parse_err(format!("{errs:?}")))?;
                per_chain.push(bundle);
            }
            bundles.insert(cat.namespace(), per_chain);
        }
        let icu = crate::format::icu_locale(&chain[0]);
        Ok(Self {
            chain,
            icu,
            bundles,
        })
    }

    /// The active (first) locale of the chain.
    pub fn locale(&self) -> &LanguageIdentifier {
        &self.chain[0]
    }

    pub fn chain(&self) -> &[LanguageIdentifier] {
        &self.chain
    }

    /// The message, taken from the first chain entry that has it; `ns.id`
    /// when none does (logged once in debug builds).
    pub fn format(&self, ns: &str, id: &str, args: Option<&FluentArgs>) -> Cow<'_, str> {
        if let Some(bundles) = self.bundles.get(ns) {
            for bundle in bundles {
                let Some(pattern) = bundle.get_message(id).and_then(|m| m.value()) else {
                    continue;
                };
                let mut errors = Vec::new();
                let out = bundle.format_pattern(pattern, args, &mut errors);
                if !errors.is_empty() {
                    report_errors(ns, id, errors.len());
                }
                return out;
            }
        }
        report_missing(ns, id);
        Cow::Owned(format!("{ns}.{id}"))
    }

    /// Whether some chain entry of namespace `ns` has message `id`.
    pub fn has(&self, ns: &str, id: &str) -> bool {
        self.bundles
            .get(ns)
            .is_some_and(|bs| bs.iter().any(|b| b.has_message(id)))
    }

    pub fn text(&self, t: &LocalizedText) -> Cow<'_, str> {
        if t.args.is_empty() {
            return self.format(&t.ns, &t.id, None);
        }
        let mut args = FluentArgs::new();
        for (k, v) in &t.args {
            args.set(k.to_string(), v.to_value());
        }
        self.format(&t.ns, &t.id, Some(&args))
    }
}

/// Anything that can hand out the active localizer (the app global later).
pub trait Translate {
    fn localizer(&self) -> &Localizer;
}

impl Translate for Localizer {
    fn localizer(&self) -> &Localizer {
        self
    }
}

#[cfg(debug_assertions)]
fn report_missing(ns: &str, id: &str) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Ok(mut s) = seen.lock()
        && s.insert(format!("{ns}.{id}"))
    {
        eprintln!("i18n: missing message {ns}.{id}");
    }
}

/// Fluent errors (a missing argument renders as `{$x}`), logged once per message in debug builds.
#[cfg(debug_assertions)]
fn report_errors(ns: &str, id: &str, n: usize) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Ok(mut s) = seen.lock()
        && s.insert(format!("{ns}.{id}"))
    {
        eprintln!("i18n: {n} formatting error(s) in {ns}.{id}");
    }
}

#[cfg(not(debug_assertions))]
fn report_errors(_ns: &str, _id: &str, _n: usize) {}

#[cfg(not(debug_assertions))]
fn report_missing(_ns: &str, _id: &str) {}

/// Accented, elongated and bracketed, so truncation and hard-coded text show
/// up. fluent-pseudo elongates with plain vowels, so every ASCII letter left
/// over is accented too: no plain ASCII letters survive outside placeables.
fn pseudo(s: &str) -> Cow<'_, str> {
    const LOWER: &[char] = &[
        'á', 'ƀ', 'ç', 'ð', 'é', 'ƒ', 'ĝ', 'ĥ', 'í', 'ĵ', 'ķ', 'ļ', 'ɱ', 'ñ', 'ó', 'þ', 'ƣ', 'ŕ',
        'š', 'ţ', 'ú', 'ṽ', 'ŵ', 'ẋ', 'ý', 'ž',
    ];
    const UPPER: &[char] = &[
        'Á', 'Ɓ', 'Ç', 'Ð', 'É', 'Ƒ', 'Ĝ', 'Ĥ', 'Í', 'Ĵ', 'Ķ', 'Ļ', 'Ṁ', 'Ñ', 'Ó', 'Þ', 'Ƣ', 'Ŕ',
        'Š', 'Ţ', 'Ú', 'Ṽ', 'Ŵ', 'Ẋ', 'Ý', 'Ž',
    ];
    let t = fluent_pseudo::transform_dom(s, false, true, true);
    let mut out: String = t
        .chars()
        .map(|c| match c {
            'a'..='z' => LOWER[c as usize - 'a' as usize],
            'A'..='Z' => UPPER[c as usize - 'A' as usize],
            _ => c,
        })
        .collect();
    // fluent-pseudo elongates short words more than long ones; guarantee 30%.
    let want = s.chars().count() * 13 / 10 + 1;
    let have = out.chars().count();
    out.extend(std::iter::repeat_n('·', want.saturating_sub(have)));
    Cow::Owned(out)
}
