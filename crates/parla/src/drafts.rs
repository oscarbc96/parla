//! Drafts and sidebar sections of a workspace session: the Drafts view's
//! list and delete, and the collapsed sidebar sections Parla keeps per
//! workspace in the store.

use components::{DraftItem, DraftKind, DraftsSource, SidebarSection};
use domain::{ConversationKind, DraftKey};
use gpui::{App, Context};

use crate::session::{SessionEvent, WorkspaceSession};

impl WorkspaceSession {
    /// Reads the saved drafts (a store without a cache paint: the SQLite path
    /// reads them with the cache). A draft saved meanwhile wins.
    pub(crate) fn load_drafts(&self, cx: &mut Context<Self>) {
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(cx, async move { st.drafts(&ws).await }, |s, r, cx| {
            let Ok(all) = r else { return };
            if s.merge_loaded_drafts(all) {
                cx.notify();
            }
        });
    }

    /// Reads the collapsed sidebar sections from the store (once per open).
    pub(crate) fn load_collapsed(&self, cx: &mut Context<Self>) {
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(
            cx,
            async move { st.sidebar_collapsed(&ws).await },
            |s, r, cx| {
                let Ok(keys) = r else { return };
                let mut all: std::collections::BTreeSet<SidebarSection> = keys
                    .iter()
                    .filter_map(|k| SidebarSection::from_key(k))
                    .collect();
                // What the user toggled before the load arrived wins over it.
                for t in &s.collapsed_touched {
                    if s.collapsed.contains(t) {
                        all.insert(*t);
                    } else {
                        all.remove(t);
                    }
                }
                if all != s.collapsed {
                    s.collapsed = all;
                    s.invalidate_rows();
                    cx.notify();
                }
            },
        );
    }

    /// Adds the drafts a store load found. A draft saved or deleted this
    /// session meanwhile wins over the stored one. True when anything changed.
    pub(crate) fn merge_loaded_drafts(&mut self, all: Vec<store::Draft>) -> bool {
        let mut changed = false;
        for (k, b, at) in all {
            if self.drafts_deleted.contains(&k) {
                continue;
            }
            if let std::collections::hash_map::Entry::Vacant(e) = self.drafts.entry(k) {
                e.insert((b, at));
                changed = true;
            }
        }
        if changed {
            self.invalidate_rows();
        }
        changed
    }

    /// Deletes a draft (memory and store); a load answering later keeps it
    /// deleted.
    pub fn forget_draft(&mut self, key: DraftKey, cx: &mut Context<Self>) {
        self.save_draft(key, domain::ComposedText::plain(""), cx)
            .detach();
    }

    /// What a composer on screen holds now, shown in the Drafts view while it
    /// is open. Memory only: the store gets it when the composer is left.
    pub fn set_live_draft(
        &mut self,
        key: DraftKey,
        body: domain::ComposedText,
        cx: &mut Context<Self>,
    ) {
        if body.text.trim().is_empty() {
            if self.drafts.remove(&key).is_some() {
                self.invalidate_rows();
                cx.notify();
            }
            return;
        }
        if self.drafts.get(&key).map(|(b, _)| b) == Some(&body) {
            return;
        }
        self.drafts_deleted.remove(&key);
        self.drafts.insert(key, (body, components::now_secs(cx)));
        self.invalidate_rows();
        cx.notify();
    }

    /// Drops the folded state of a deleted custom section (memory and
    /// store), so a new section that reuses its id opens unfolded.
    pub fn forget_section_fold(&mut self, id: u32, cx: &mut Context<Self>) {
        let section = SidebarSection::Custom(id);
        let was = self.collapsed.remove(&section);
        self.collapsed_touched.insert(section);
        if was {
            self.invalidate_rows();
            cx.notify();
        }
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(
            cx,
            async move {
                st.sidebar_collapsed_put(&ws, section.key().to_string(), false)
                    .await
            },
            |_, r, _| {
                if r.is_err() {
                    crate::log::warn("sidebar section save failed");
                }
            },
        );
    }

    /// Collapses or expands `section` and keeps the choice in the store.
    pub fn toggle_section_now(&mut self, section: SidebarSection, cx: &mut Context<Self>) {
        if !section.collapsible() {
            return;
        }
        let collapsed = if self.collapsed.remove(&section) {
            false
        } else {
            self.collapsed.insert(section);
            true
        };
        self.collapsed_touched.insert(section);
        self.invalidate_rows();
        cx.notify();
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(
            cx,
            async move {
                st.sidebar_collapsed_put(&ws, section.key().to_string(), collapsed)
                    .await
            },
            |_, r, _| {
                if r.is_err() {
                    crate::log::warn("sidebar section save failed");
                }
            },
        );
    }
}

impl DraftsSource for WorkspaceSession {
    fn draft_items(&self, cx: &App) -> Vec<DraftItem> {
        let l = i18n::Translate::localizer(cx);
        let index = self.member_index();
        self.drafts
            .iter()
            .filter(|(_, (b, _))| !b.text.trim().is_empty())
            .map(|(key, (body, edited))| {
                let conv = self.state.conversations.get(&key.conversation);
                let (label, kind) = match conv {
                    Some(c) => {
                        let name =
                            components::sidebar::conversation_label(&self.state, &index, c, l);
                        match c.kind {
                            ConversationKind::Channel => (format!("#{name}"), DraftKind::Channel),
                            ConversationKind::PrivateChannel => {
                                (name.to_string(), DraftKind::PrivateChannel)
                            }
                            ConversationKind::Direct | ConversationKind::Group => {
                                (name.to_string(), DraftKind::Direct)
                            }
                        }
                    }
                    // An unknown conversation shows nothing, never its raw id.
                    None => (String::new(), DraftKind::Channel),
                };
                DraftItem {
                    key: key.clone(),
                    conversation: label,
                    kind: if key.thread.is_some() {
                        DraftKind::Thread
                    } else {
                        kind
                    },
                    text: body.text.to_string(),
                    edited: *edited,
                }
            })
            .collect()
    }

    fn delete_draft(&mut self, key: DraftKey, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            s.forget_draft(key.clone(), cx);
            cx.emit(SessionEvent::DraftDeleted(key));
        });
    }
}
