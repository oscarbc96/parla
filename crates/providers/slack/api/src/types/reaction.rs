use serde::Deserialize;

/// A reaction on a message as Slack sends it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ReactionRaw {
    pub name: String,
    pub count: u32,
    pub users: Vec<String>,
}
