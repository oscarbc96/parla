use std::sync::OnceLock;

use serde::Deserialize;
use unic_langid::LanguageIdentifier;

/// The pseudo-locale: selectable in every build, never listed in `locales.toml`.
pub const PSEUDO: &str = "en-XA";

#[derive(Debug, thiserror::Error)]
pub enum LocalesError {
    #[error("invalid locales.toml: {0}")]
    Toml(String),
    #[error("invalid locale tag {0:?}")]
    BadTag(String),
    #[error("duplicate locale {0}")]
    Duplicate(String),
    #[error("source locale {0} is not listed")]
    MissingSource(String),
    #[error("locale {0} has an empty name")]
    EmptyName(String),
    #[error("the pseudo-locale en-XA must not be listed")]
    Pseudo,
}

/// One supported locale; `name` is its endonym.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleInfo {
    pub code: LanguageIdentifier,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleSet {
    pub source: LanguageIdentifier,
    pub locales: Vec<LocaleInfo>,
}

#[derive(Deserialize)]
struct RawSet {
    source: String,
    locales: Vec<RawLocale>,
}

#[derive(Deserialize)]
struct RawLocale {
    code: String,
    name: String,
}

impl LocaleSet {
    /// The set embedded from `i18n/locales.toml`, parsed once.
    pub fn embedded() -> &'static LocaleSet {
        static SET: OnceLock<LocaleSet> = OnceLock::new();
        SET.get_or_init(|| {
            LocaleSet::parse(include_str!("../../../i18n/locales.toml"))
                .expect("i18n/locales.toml is valid")
        })
    }

    pub fn parse(toml: &str) -> Result<LocaleSet, LocalesError> {
        let raw: RawSet = ::toml::from_str(toml).map_err(|e| LocalesError::Toml(e.to_string()))?;
        let tag = |s: &str| {
            s.parse::<LanguageIdentifier>()
                .map_err(|_| LocalesError::BadTag(s.to_string()))
        };
        let source = tag(&raw.source)?;
        let mut locales: Vec<LocaleInfo> = Vec::with_capacity(raw.locales.len());
        for l in raw.locales {
            let code = tag(&l.code)?;
            if code == PSEUDO {
                return Err(LocalesError::Pseudo);
            }
            if l.name.trim().is_empty() {
                return Err(LocalesError::EmptyName(l.code));
            }
            if locales.iter().any(|x| x.code == code) {
                return Err(LocalesError::Duplicate(l.code));
            }
            locales.push(LocaleInfo { code, name: l.name });
        }
        if !locales.iter().any(|l| l.code == source) {
            return Err(LocalesError::MissingSource(raw.source));
        }
        Ok(LocaleSet { source, locales })
    }

    pub fn get(&self, code: &LanguageIdentifier) -> Option<&LocaleInfo> {
        self.locales.iter().find(|l| &l.code == code)
    }
}
