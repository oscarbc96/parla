use std::borrow::Cow;

/// One crate's embedded Fluent files (`<code>.ftl`) under a namespace.
/// Built by [`namespace!`](crate::namespace); debug builds read the folder
/// from disk, release builds embed it.
pub struct Catalog {
    namespace: &'static str,
    get: fn(&str) -> Option<Cow<'static, [u8]>>,
}

impl Catalog {
    pub const fn new(namespace: &'static str, get: fn(&str) -> Option<Cow<'static, [u8]>>) -> Self {
        Self { namespace, get }
    }

    pub fn namespace(&self) -> &'static str {
        self.namespace
    }

    /// The FTL source for a locale code, if the catalog has that file.
    pub fn source(&self, code: &str) -> Option<Cow<'static, str>> {
        let bytes = (self.get)(&format!("{code}.ftl"))?;
        match bytes {
            Cow::Borrowed(b) => std::str::from_utf8(b).ok().map(Cow::Borrowed),
            Cow::Owned(v) => String::from_utf8(v).ok().map(Cow::Owned),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("catalog {ns}: no source file for {code}")]
    MissingSource { ns: String, code: String },
    #[error("catalog {ns}/{code}: {message}")]
    Parse {
        ns: String,
        code: String,
        message: String,
    },
    #[error("empty locale chain")]
    EmptyChain,
}
