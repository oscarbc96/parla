use std::sync::Arc;

use fluent_bundle::FluentValue;

/// A message argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Str(Arc<str>),
    Num(f64),
}

impl Arg {
    #[doc(hidden)]
    pub fn to_value(&self) -> FluentValue<'static> {
        match self {
            Arg::Str(s) => FluentValue::from(s.to_string()),
            Arg::Num(n) => FluentValue::from(*n),
        }
    }
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Arg::Str(s.into())
    }
}
impl From<String> for Arg {
    fn from(s: String) -> Self {
        Arg::Str(s.into())
    }
}
impl From<&String> for Arg {
    fn from(s: &String) -> Self {
        Arg::Str(s.as_str().into())
    }
}
impl From<Arc<str>> for Arg {
    fn from(s: Arc<str>) -> Self {
        Arg::Str(s)
    }
}
impl From<f64> for Arg {
    fn from(n: f64) -> Self {
        Arg::Num(n)
    }
}
macro_rules! num_from {
    ($($t:ty),*) => {$(impl From<$t> for Arg { fn from(n: $t) -> Self { Arg::Num(n as f64) } })*};
}
num_from!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32);

/// Text produced in one crate and rendered in another (provider errors,
/// notices): a message id in a namespace plus its arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalizedText {
    pub ns: Arc<str>,
    pub id: Arc<str>,
    pub args: Vec<(Arc<str>, Arg)>,
}

impl LocalizedText {
    pub fn new(ns: &str, id: &str) -> Self {
        Self {
            ns: ns.into(),
            id: id.into(),
            args: Vec::new(),
        }
    }

    pub fn with(mut self, name: &str, value: impl Into<Arg>) -> Self {
        self.args.push((name.into(), value.into()));
        self
    }
}
