//! Workspace areas a manifest may grant. The names are
//! the ones in `parla_plugin_api::READ_AREAS` and `WRITE_AREAS`; the parser
//! checks a name against those lists, and a test keeps these enums in step.

use parla_plugin_api::{READ_AREAS, WRITE_AREAS};

macro_rules! areas {
    ($(#[$doc:meta])* $name:ident, $list:ident, { $($variant:ident => $text:literal,)* }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            /// Every area, in the order of the API list.
            pub const ALL: &[$name] = &[$($name::$variant,)*];

            /// The manifest spelling, as in the API list.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }

            /// The area named `name`, if the API list has it.
            pub fn from_name(name: &str) -> Option<Self> {
                if !$list.contains(&name) {
                    return None;
                }
                Self::ALL.iter().copied().find(|a| a.as_str() == name)
            }
        }

        /// Stored by its manifest spelling (a grant in `app.db`).
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let name = <std::borrow::Cow<'de, str>>::deserialize(d)?;
                Self::from_name(&name).ok_or_else(|| {
                    serde::de::Error::custom(format_args!("`{name}` is not an area"))
                })
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

areas!(
    /// An area a plugin may read (`workspace_read`).
    ReadArea, READ_AREAS, {
        Workspace => "workspace:read",
        Conversations => "conversations:read",
        Messages => "messages:read",
        History => "history:read",
        Dms => "dms:read",
        People => "people:read",
        Search => "search:read",
        Files => "files:read",
        Reminders => "reminders:read",
        Scheduled => "scheduled:read",
        Saved => "saved:read",
        Pins => "pins:read",
        Status => "status:read",
    }
);

areas!(
    /// An area a plugin may write as the user (`workspace_write`).
    WriteArea, WRITE_AREAS, {
        Messages => "messages:write",
        MessagesEdit => "messages:edit",
        Reactions => "reactions:write",
        Reminders => "reminders:write",
        Scheduled => "scheduled:write",
        Files => "files:write",
        Status => "status:write",
        Saved => "saved:write",
        Pins => "pins:write",
        ReadState => "read_state:write",
    }
);
