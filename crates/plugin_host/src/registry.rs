//! The installed set: installed plugins and their registered contributions
//! indexed by kind.
//!
//! A plugin is in the set while installed, whatever its state; its
//! contributions are indexed only while registered (Installed, Active,
//! Unloaded). Registering or removing one plugin costs
//! O(its contributions · log n); nothing scans the other plugins.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::lifecycle::Effect;
use crate::manifest::{Manifest, PluginId};

/// The kinds of contribution a manifest declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContributionKind {
    Command,
    Slash,
    MessageAction,
    ComposerTransform,
    Renderer,
    MediaProvider,
    Panel,
    McpTool,
    /// One per hook event; the id is the event's manifest name.
    Hook,
    NotificationFilter,
    /// One per theme file.
    Theme,
    IconPack,
}

impl ContributionKind {
    pub const ALL: [Self; 12] = [
        Self::Command,
        Self::Slash,
        Self::MessageAction,
        Self::ComposerTransform,
        Self::Renderer,
        Self::MediaProvider,
        Self::Panel,
        Self::McpTool,
        Self::Hook,
        Self::NotificationFilter,
        Self::Theme,
        Self::IconPack,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

/// One registered contribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContributionRef<'a> {
    pub plugin: &'a PluginId,
    pub kind: ContributionKind,
    /// The contribution's id (a slash command's name, an MCP tool's name,
    /// a hook's event, a theme's file).
    pub id: &'a str,
}

struct Entry {
    manifest: Arc<Manifest>,
    registered: bool,
}

/// Installed plugins and their contributions, indexed by kind.
#[derive(Default)]
pub struct InstalledSet {
    plugins: BTreeMap<PluginId, Entry>,
    by_kind: [BTreeSet<(PluginId, String)>; ContributionKind::ALL.len()],
}

impl InstalledSet {
    /// Adds an installed plugin, contributions not registered. Replacing a
    /// plugin (an update) keeps its registration and reindexes it.
    pub fn insert(&mut self, manifest: Arc<Manifest>) {
        let id = manifest.id.clone();
        let registered = self.is_registered(&id);
        self.unregister(&id);
        self.plugins.insert(
            id.clone(),
            Entry {
                manifest,
                registered: false,
            },
        );
        if registered {
            self.register(&id);
        }
    }

    /// Removes an uninstalled plugin and its contributions.
    pub fn remove(&mut self, id: &PluginId) {
        self.unregister(id);
        self.plugins.remove(id);
    }

    pub fn get(&self, id: &PluginId) -> Option<&Arc<Manifest>> {
        self.plugins.get(id).map(|e| &e.manifest)
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Installed plugins, by id.
    pub fn manifests(&self) -> impl Iterator<Item = &Arc<Manifest>> {
        self.plugins.values().map(|e| &e.manifest)
    }

    pub fn is_registered(&self, id: &PluginId) -> bool {
        self.plugins.get(id).is_some_and(|e| e.registered)
    }

    /// Indexes the plugin's contributions. No-op when not installed or
    /// already registered.
    pub fn register(&mut self, id: &PluginId) {
        let Some(entry) = self.plugins.get_mut(id) else {
            return;
        };
        if entry.registered {
            return;
        }
        entry.registered = true;
        for (kind, key) in keys(&entry.manifest) {
            self.by_kind[kind.index()].insert((id.clone(), key));
        }
    }

    /// Removes the plugin's contributions from the index; it stays
    /// installed.
    pub fn unregister(&mut self, id: &PluginId) {
        let Some(entry) = self.plugins.get_mut(id) else {
            return;
        };
        if !entry.registered {
            return;
        }
        entry.registered = false;
        for (kind, key) in keys(&entry.manifest) {
            self.by_kind[kind.index()].remove(&(id.clone(), key));
        }
    }

    /// Applies the contribution effects of a lifecycle transition; the
    /// others are the host's to carry out.
    pub fn apply(&mut self, id: &PluginId, effects: &[Effect]) {
        for e in effects {
            match e {
                Effect::RegisterContributions => self.register(id),
                Effect::RemoveContributions => self.unregister(id),
                _ => {}
            }
        }
    }

    /// Registered contributions of `kind`, by plugin id then contribution
    /// id.
    pub fn contributions(
        &self,
        kind: ContributionKind,
    ) -> impl Iterator<Item = ContributionRef<'_>> {
        self.by_kind[kind.index()]
            .iter()
            .map(move |(plugin, id)| ContributionRef { plugin, kind, id })
    }
}

/// Every contribution of a manifest as `(kind, id)`.
fn keys(m: &Manifest) -> Vec<(ContributionKind, String)> {
    use ContributionKind as K;
    let c = &m.contributions;
    let mut out = Vec::new();
    out.extend(c.commands.iter().map(|x| (K::Command, x.id.clone())));
    out.extend(c.slash.iter().map(|x| (K::Slash, x.name.clone())));
    out.extend(
        c.message_actions
            .iter()
            .map(|x| (K::MessageAction, x.id.clone())),
    );
    out.extend(
        c.composer_transforms
            .iter()
            .map(|x| (K::ComposerTransform, x.id.clone())),
    );
    out.extend(c.renderers.iter().map(|x| (K::Renderer, x.id.clone())));
    out.extend(
        c.media_providers
            .iter()
            .map(|x| (K::MediaProvider, x.id.clone())),
    );
    out.extend(c.panels.iter().map(|x| (K::Panel, x.id.clone())));
    out.extend(c.mcp_tools.iter().map(|x| (K::McpTool, x.name.clone())));
    if let Some(h) = &c.hooks {
        out.extend(h.events.iter().map(|e| (K::Hook, e.as_str().to_owned())));
        if h.notification_filter {
            out.push((K::NotificationFilter, "notification_filter".to_owned()));
        }
    }
    if let Some(t) = &m.themes {
        out.extend(t.files.iter().map(|f| (K::Theme, f.clone())));
    }
    out.extend(m.icon_packs.iter().map(|p| (K::IconPack, p.name.clone())));
    out
}
