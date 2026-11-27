use fluent_langneg::{LanguageIdentifier as IcuLangId, NegotiationStrategy, negotiate_languages};
use unic_langid::LanguageIdentifier;

use crate::locales::{LocaleSet, PSEUDO};

/// The user's choice: follow the OS or pin a locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalePref {
    System,
    Code(LanguageIdentifier),
}

/// Requested (setting or OS list) negotiated against the set; always ends in the source.
pub fn chain(pref: &LocalePref, os: &[String], set: &LocaleSet) -> Vec<LanguageIdentifier> {
    let mut out: Vec<LanguageIdentifier> = Vec::new();
    let requested: Vec<LanguageIdentifier> = match pref {
        LocalePref::Code(code) => {
            if *code == PSEUDO {
                out.push(code.clone());
            }
            vec![code.clone()]
        }
        LocalePref::System => os
            .iter()
            .filter_map(|s| s.replace('_', "-").parse().ok())
            .collect(),
    };
    // fluent-langneg 0.14 uses ICU4X's identifier type, not unic-langid's.
    let icu = |l: &LanguageIdentifier| l.to_string().parse::<IcuLangId>().ok();
    let requested: Vec<IcuLangId> = requested.iter().filter_map(icu).collect();
    let available: Vec<IcuLangId> = set.locales.iter().filter_map(|l| icu(&l.code)).collect();
    let default = icu(&set.source);
    let negotiated = negotiate_languages(
        &requested,
        &available,
        default.as_ref(),
        NegotiationStrategy::Filtering,
    );
    for l in negotiated {
        if let Ok(l) = l.to_string().parse::<LanguageIdentifier>()
            && !out.contains(&l)
        {
            out.push(l);
        }
    }
    out.retain(|l| *l != set.source);
    out.push(set.source.clone());
    out
}
