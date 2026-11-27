//! Custom sidebar sections are kept in the workspace's settings file, and the
//! sidebar popover writes the same `settings.json` keys as the Appearance page.

use components::SidebarOption;
use parla::sidebar_sections::{option_edit, write_sections};
use settings::{ConfigPaths, CustomSection, Density, SettingsStore, SidebarSort, WorkspaceKey};

fn focus() -> CustomSection {
    CustomSection {
        id: 1,
        name: "Focus".into(),
        conversations: vec!["C1".into()],
    }
}

#[test]
fn sections_go_to_the_workspace_file_and_keep_its_other_keys() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    let key = WorkspaceKey::new("fake", "w1");
    // A first write creates the folder and the file.
    assert!(write_sections(&paths, &key, &[focus()]).unwrap());
    let file = paths.workspace_file(&key);
    std::fs::write(
        &file,
        "{\n  // my colour\n  \"accent\": \"#336699\",\n  \"sidebar\": { \"sections\": [] }\n}\n",
    )
    .unwrap();
    assert!(write_sections(&paths, &key, &[focus()]).unwrap());
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("// my colour"), "{text}");
    let store = SettingsStore::load(paths.clone());
    let s = store.for_workspace(&key);
    assert_eq!(s.sidebar.sections, vec![focus()]);
    assert_eq!(s.accent.as_deref(), Some("#336699"));
    // Other workspaces and the user file do not see them.
    assert!(
        store
            .for_workspace(&WorkspaceKey::new("fake", "w2"))
            .sidebar
            .sections
            .is_empty()
    );
    assert!(store.current().sidebar.sections.is_empty());
}

#[test]
fn popover_options_use_the_appearance_keys() {
    assert_eq!(
        option_edit(SidebarOption::ShowUnreads(false)),
        (
            &["sidebar", "show_unreads_section"][..],
            "false".to_string()
        )
    );
    assert_eq!(
        option_edit(SidebarOption::Sort(SidebarSort::Recent)),
        (&["sidebar", "sort"][..], "\"recent\"".to_string())
    );
    assert_eq!(
        option_edit(SidebarOption::Density(Density::Cozy)),
        (&["density"][..], "\"cozy\"".to_string())
    );
}

mod app {
    use std::sync::Arc;

    use components::{SidebarRow, SidebarSource};
    use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, uid};
    use gpui::TestAppContext;
    use parla::providers::ProviderRegistry;
    use parla::root_view::RootView;
    use settings::{ConfigPaths, SettingsStore, SidebarSort};
    use store::{MemoryStore, Store};

    fn headers_and_rows(rows: &[SidebarRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                SidebarRow::Header { label, .. } => format!("#{label}"),
                SidebarRow::Conversation { label, .. } => label.to_string(),
                SidebarRow::ShowMore { label, .. } => label.to_string(),
            })
            .collect()
    }

    #[gpui::test]
    async fn the_row_menu_moves_a_conversation_into_a_section(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::at(dir.path());
        let p = Arc::new(FakeProvider::new("w1", uid("u1")));
        p.add_conversation(conv("C1", "general"));
        p.add_conversation(conv("C2", "random"));
        p.add_member(member("u1", "Me"));
        let ws = domain::ChatProvider::workspace(&*p).clone();
        super::write_sections(
            &paths,
            &ws,
            &[settings::CustomSection {
                id: 1,
                name: "Focus".into(),
                conversations: Vec::new(),
            }],
        )
        .unwrap();
        cx.update(|cx| {
            parla::init_without_watcher(cx, paths.clone());
            i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        });
        let mut registry = ProviderRegistry::new();
        registry.add(FakeFactory::default().with_provider(p));
        let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
        let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
        cx.run_until_parked();
        let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
        let sidebar = view.read_with(cx, |v, _| v.sidebar().unwrap().clone());
        assert_eq!(
            headers_and_rows(&session.read_with(cx, SidebarSource::rows)),
            ["#Focus", "#Channels", "general", "random"]
        );
        sidebar.update(cx, |_, cx| {
            cx.emit(components::SidebarEvent::ContextMenu {
                conversation: cid("C1"),
                position: gpui::point(gpui::px(100.0), gpui::px(100.0)),
            })
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let labels = view.read_with(cx, |v, cx| {
            v.notify_menu()
                .unwrap()
                .read(cx)
                .extra_labels()
                .into_iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        });
        let ix = labels
            .iter()
            .position(|l| l == "Move to Focus")
            .unwrap_or_else(|| panic!("no Move to: {labels:?}"));
        let b = cx
            .debug_bounds(Box::leak(format!("notify-extra-{ix}").into_boxed_str()))
            .expect("move entry");
        cx.simulate_click(b.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            headers_and_rows(&session.read_with(cx, SidebarSource::rows)),
            ["#Focus", "general", "#Channels", "random"]
        );
        let text = std::fs::read_to_string(paths.workspace_file(&ws)).unwrap();
        assert!(text.contains("\"C1\""), "{text}");
        // Back to Channels from the same menu.
        sidebar.update(cx, |_, cx| {
            cx.emit(components::SidebarEvent::ContextMenu {
                conversation: cid("C1"),
                position: gpui::point(gpui::px(100.0), gpui::px(100.0)),
            })
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let labels = view.read_with(cx, |v, cx| {
            v.notify_menu()
                .unwrap()
                .read(cx)
                .extra_labels()
                .into_iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        });
        assert!(!labels.iter().any(|l| l == "Move to Focus"), "{labels:?}");
        assert!(labels.iter().any(|l| l == "Move to Channels"), "{labels:?}");
        // The popover's sort goes to settings.json and applies at once.
        session.update(cx, |s, cx| {
            s.sidebar_option_now(components::SidebarOption::Sort(SidebarSort::Recent), cx)
        });
        cx.run_until_parked();
        let sort = cx.update(|_, cx| cx.global::<SettingsStore>().current().sidebar.sort);
        assert_eq!(sort, SidebarSort::Recent);
    }

    async fn start<'a>(
        cx: &'a mut TestAppContext,
        paths: &ConfigPaths,
        p: Arc<FakeProvider>,
    ) -> (
        gpui::Entity<RootView>,
        &'a mut gpui::VisualTestContext,
        Arc<MemoryStore>,
    ) {
        cx.update(|cx| {
            parla::init_without_watcher(cx, paths.clone());
            i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        });
        let mut registry = ProviderRegistry::new();
        registry.add(FakeFactory::default().with_provider(p));
        let store = Arc::new(MemoryStore::default());
        let st: Arc<dyn Store> = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
        cx.run_until_parked();
        (view, cx, store)
    }

    fn provider() -> Arc<FakeProvider> {
        let p = Arc::new(FakeProvider::new("w1", uid("u1")));
        let mut a = conv("C1", "general");
        a.latest = Some(domain::test_support::ts("1.000000"));
        let mut b = conv("C2", "random");
        b.latest = Some(domain::test_support::ts("9.000000"));
        p.add_conversation(a);
        p.add_conversation(b);
        p.add_member(member("u1", "Me"));
        p
    }

    #[gpui::test]
    async fn a_deleted_section_takes_its_fold_with_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::at(dir.path());
        let p = provider();
        let ws = domain::ChatProvider::workspace(&*p).clone();
        let (view, cx, store) = start(cx, &paths, p).await;
        let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
        let op = |name: &str| components::SectionOp::Create { name: name.into() };
        session.update(cx, |s, cx| s.section_op_now(op("Focus"), cx));
        cx.run_until_parked();
        let folded = components::SidebarSection::Custom(1);
        session.update(cx, |s, cx| s.toggle_section_now(folded, cx));
        cx.run_until_parked();
        assert!(
            store
                .sidebar_collapsed(&ws)
                .await
                .unwrap()
                .contains("section:1")
        );
        session.update(cx, |s, cx| {
            s.section_op_now(components::SectionOp::Delete { id: 1 }, cx)
        });
        cx.run_until_parked();
        assert!(store.sidebar_collapsed(&ws).await.unwrap().is_empty());
        // A new section gets id 1 again and opens unfolded.
        session.update(cx, |s, cx| s.section_op_now(op("Clients"), cx));
        cx.run_until_parked();
        let rows = session.read_with(cx, SidebarSource::rows);
        assert!(rows.iter().any(|r: &SidebarRow| matches!(
            r,
            SidebarRow::Header { section, collapsed: false, .. } if *section == folded
        )));
    }

    #[gpui::test]
    async fn only_the_workspace_layer_is_copied_into_the_workspace_file(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::at(dir.path());
        std::fs::write(
            &paths.settings,
            r#"{ "sidebar": { "sections": [{ "id": 7, "name": "Everywhere", "conversations": [] }] } }"#,
        )
        .unwrap();
        let p = provider();
        let ws = domain::ChatProvider::workspace(&*p).clone();
        let (view, cx, _) = start(cx, &paths, p).await;
        let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
        session.update(cx, |s, cx| {
            s.section_op_now(
                components::SectionOp::Create {
                    name: "Mine".into(),
                },
                cx,
            )
        });
        cx.run_until_parked();
        let got = parla::sidebar_sections::workspace_sections(&paths, &ws);
        assert_eq!(
            got,
            vec![settings::CustomSection {
                id: 1,
                name: "Mine".into(),
                conversations: vec![],
            }]
        );
    }

    #[gpui::test]
    async fn popover_options_apply_to_the_rows_at_once(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::at(dir.path());
        // A reload applies `locale`: pin English so the headers read the same.
        std::fs::write(&paths.settings, r#"{ "locale": "en" }"#).unwrap();
        let p = provider();
        let ws = domain::ChatProvider::workspace(&*p).clone();
        let (view, cx, _) = start(cx, &paths, p).await;
        let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
        let rows = |cx: &mut gpui::VisualTestContext| {
            headers_and_rows(&session.read_with(cx, SidebarSource::rows))
        };
        assert_eq!(rows(cx), ["#Channels", "general", "random"]);
        session.update(cx, |s, cx| {
            s.sidebar_option_now(components::SidebarOption::Sort(SidebarSort::Recent), cx)
        });
        cx.run_until_parked();
        assert_eq!(rows(cx), ["#Channels", "random", "general"]);
        assert_eq!(
            session.read_with(cx, |s, cx| s.sidebar_options(cx)),
            (true, SidebarSort::Recent)
        );
        // A workspace file that sets the sort wins; the popover writes there.
        std::fs::create_dir_all(paths.workspace_file(&ws).parent().unwrap()).unwrap();
        std::fs::write(
            paths.workspace_file(&ws),
            r#"{ "sidebar": { "sort": "alphabetical" } }"#,
        )
        .unwrap();
        cx.update(|_, cx| {
            cx.global_mut::<SettingsStore>().note_workspace_changed();
            cx.refresh_windows();
        });
        session.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(
            session.read_with(cx, |s, cx| s.sidebar_options(cx)).1,
            SidebarSort::Alphabetical
        );
        session.update(cx, |s, cx| {
            s.sidebar_option_now(components::SidebarOption::Sort(SidebarSort::Recent), cx)
        });
        cx.run_until_parked();
        assert_eq!(
            settings::layer_value(&paths.workspace_file(&ws), &["sidebar", "sort"]),
            Some(serde_json::json!("recent"))
        );
        assert_eq!(rows(cx), ["#Channels", "random", "general"]);
    }
}
