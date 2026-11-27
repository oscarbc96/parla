//! Per-instance plugin state. `Plugin` methods are static (wit-bindgen calls
//! them without a value), so state lives outside the type. A plain `static`
//! would be shared by every test thread in a native `cargo test` run;
//! [`plugin_state!`](crate::plugin_state) keeps one value per thread
//! instead. In wasm a plugin instance runs on one thread, so it is simply
//! the instance's state; under `cargo test` each test (its own thread)
//! starts from the initial value.
//!
//! ```ignore
//! parla_plugin_sdk::plugin_state!(static SEEN: Vec<String> = Vec::new());
//! SEEN.with(|seen| seen.push(id));
//! ```

use std::cell::RefCell;
use std::thread::LocalKey;

/// A per-thread value made by [`plugin_state!`](crate::plugin_state).
pub struct State<T: 'static> {
    key: &'static LocalKey<RefCell<T>>,
}

impl<T: 'static> State<T> {
    #[doc(hidden)]
    pub const fn new(key: &'static LocalKey<RefCell<T>>) -> Self {
        Self { key }
    }

    /// Runs `f` with the value borrowed mutably. Do not call back into the
    /// same state from `f`.
    pub fn with<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R {
        self.key.with_borrow_mut(f)
    }

    /// Replaces the value, returning the old one.
    pub fn replace(&'static self, value: T) -> T {
        self.key.replace(value)
    }

    pub fn set(&'static self, value: T) {
        self.key.set(value);
    }

    pub fn get(&'static self) -> T
    where
        T: Clone,
    {
        self.key.with_borrow(T::clone)
    }

    /// Takes the value, leaving the default.
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.key.take()
    }
}

/// Declares per-instance plugin state: `plugin_state!(static NAME: Type =
/// init);`. The value is per thread: see [`State`]($crate::State).
#[macro_export]
macro_rules! plugin_state {
    ($vis:vis static $name:ident: $ty:ty = $init:expr) => {
        $vis static $name: $crate::State<$ty> = {
            ::std::thread_local! {
                static STATE: ::std::cell::RefCell<$ty> = ::std::cell::RefCell::new($init);
            }
            $crate::State::new(&STATE)
        };
    };
    ($vis:vis static $name:ident: $ty:ty = $init:expr;) => {
        $crate::plugin_state!($vis static $name: $ty = $init);
    };
}
