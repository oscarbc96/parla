use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// Typed provider extension: carries whatever the provider returns that the
/// neutral model has no field for. Only the owning provider downcasts it.
#[derive(Clone, Default)]
pub struct ProviderExt(Option<Arc<ExtInner>>);

struct ExtInner {
    value: Arc<dyn Any + Send + Sync>,
    type_name: &'static str,
}

impl ProviderExt {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn new<T: Any + Send + Sync>(v: T) -> Self {
        Self(Some(Arc::new(ExtInner {
            value: Arc::new(v),
            type_name: std::any::type_name::<T>(),
        })))
    }

    pub fn get<T: Any>(&self) -> Option<&T> {
        self.0.as_ref()?.value.downcast_ref::<T>()
    }
}

impl fmt::Debug for ProviderExt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(i) => write!(f, "ProviderExt({})", i.type_name),
            None => f.write_str("ProviderExt(none)"),
        }
    }
}
