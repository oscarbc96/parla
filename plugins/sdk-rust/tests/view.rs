//! View builders: nested builders flatten into the WIT flat
//! node list, and `validate` applies the host's eight tree rules.

use parla_plugin_sdk::Error;
use parla_plugin_sdk::view::{
    self, ButtonLook, NodeKind, Tone, ViewTree, button, callout, card, column, empty, heading,
    icon, image, input, list, node, row, split, stat, table, tabs, text, textarea,
};

#[test]
fn view_builder_produces_valid_flat_tree() {
    let tree = column()
        .child(heading("Standup"))
        .child(row().child(text("Ready?")).child(button("go", "Go")))
        .child(text("Bye"))
        .build();

    let parents: Vec<Option<u32>> = tree.nodes.iter().map(|n| n.parent).collect();
    assert_eq!(
        parents,
        vec![None, Some(0), Some(0), Some(2), Some(2), Some(0)]
    );
    assert!(matches!(tree.nodes[0].kind, NodeKind::Column));
    assert!(matches!(&tree.nodes[1].kind, NodeKind::Heading(h) if h == "Standup"));
    assert!(matches!(&tree.nodes[3].kind, NodeKind::Text(spans) if spans[0].text == "Ready?"));
    assert_eq!(tree.nodes[4].id.as_deref(), Some("go"));
    assert!(matches!(&tree.nodes[4].kind, NodeKind::Button(b) if b.label == "Go"));
    view::validate(&tree).unwrap();
}

fn invalid(tree: &ViewTree) -> String {
    match view::validate(tree) {
        Err(Error::Invalid(why)) => why,
        other => panic!("expected invalid, got {other:?}"),
    }
}

#[test]
fn validate_rejects_children_on_a_leaf() {
    let tree = text("leaf").child(text("child")).build();
    assert!(invalid(&tree).contains("children"));
}

#[test]
fn validate_rejects_duplicate_ids() {
    let tree = column()
        .child(button("a", "A"))
        .child(button("a", "B"))
        .build();
    assert!(invalid(&tree).contains("unique"));
}

#[test]
fn validate_rejects_interactive_node_without_id() {
    let mut tree = column().child(button("a", "A")).build();
    tree.nodes[1].id = None;
    assert!(invalid(&tree).contains("id"));
}

#[test]
fn validate_rejects_bad_parent_order() {
    let mut tree = column().child(text("x")).build();
    tree.nodes[1].parent = Some(1);
    assert!(invalid(&tree).contains("parent"));
    tree.nodes[1].parent = None;
    assert!(invalid(&tree).contains("root"));
}

#[test]
fn validate_checks_tabs_and_split_shapes() {
    let ok = tabs("t", ["One", "Two"])
        .child(text("1"))
        .child(text("2"))
        .build();
    view::validate(&ok).unwrap();
    let short = tabs("t", ["One", "Two"]).child(text("1")).build();
    assert!(invalid(&short).contains("tabs"));
    let lopsided = split().child(card()).build();
    assert!(invalid(&lopsided).contains("split"));
}

#[test]
fn validate_limits_node_count() {
    let mut col = column();
    for i in 0..2_000 {
        col = col.child(input(format!("f{i}"), "Field"));
    }
    match view::validate(&col.build()) {
        Err(Error::Limit(_)) => {}
        other => panic!("expected limit, got {other:?}"),
    }
}

#[test]
fn node_escape_hatch_takes_any_kind() {
    let tree = column().child(node(NodeKind::Spinner)).build();
    assert!(matches!(tree.nodes[1].kind, NodeKind::Spinner));
    view::validate(&tree).unwrap();
}

#[test]
fn typed_setters_fill_their_fields() {
    let tree = column()
        .child(button("go", "Go").look(ButtonLook::Primary).disabled(true))
        .child(input("name", "Name").placeholder("Ada").value("A"))
        .child(textarea("notes", "Notes"))
        .child(stat("Open", "3").note("since Monday"))
        .child(callout(Tone::Warning, "Careful").title("Heads up"))
        .child(empty("Nothing").text("Add one"))
        .child(
            table(["a", "b"], [["1", "2"]])
                .id("t")
                .selectable(true)
                .load_more(true),
        )
        .child(list().id("l").load_more(true).child(text("row")))
        .build();
    let k: Vec<&NodeKind> = tree.nodes.iter().map(|n| &n.kind).collect();
    assert!(matches!(k[1], NodeKind::Button(b) if b.look == ButtonLook::Primary && b.disabled));
    assert!(
        matches!(k[2], NodeKind::Input(t) if t.placeholder.as_deref() == Some("Ada") && t.value == "A")
    );
    assert!(matches!(k[4], NodeKind::Stat(s) if s.note.as_deref() == Some("since Monday")));
    assert!(matches!(k[5], NodeKind::Callout(c) if c.title.as_deref() == Some("Heads up")));
    assert!(matches!(k[6], NodeKind::Empty(e) if e.text.as_deref() == Some("Add one")));
    assert!(
        matches!(k[7], NodeKind::Table(t) if t.selectable && t.load_more && t.rows == [["1", "2"]])
    );
    assert!(matches!(k[8], NodeKind::List(l) if l.load_more && !l.selectable));
    view::validate(&tree).unwrap();
}

#[test]
fn size_follows_the_wit_measure() {
    // 16 per node; strings by UTF-8 length; 4 per list element.
    let tree = column().child(text("hé")).child(button("go", "Go")).build();
    assert_eq!(view::size(&tree), 16 + (16 + 4 + 3) + (16 + 2 + 2));
}

#[test]
fn validate_limits_bytes_at_the_boundary() {
    // One text node: 16 + 4 + n bytes.
    let at = view::MAX_BYTES - 20;
    view::validate(&text("a".repeat(at)).build()).unwrap();
    match view::validate(&text("a".repeat(at + 1)).build()) {
        Err(Error::Limit(why)) => assert!(why.starts_with("rule 8:")),
        other => panic!("expected limit, got {other:?}"),
    }
}

#[test]
fn validate_requires_accessible_names() {
    assert!(invalid(&column().child(input("n", " ")).build()).starts_with("rule 9:"));
    assert!(invalid(&row().child(button("b", "")).build()).starts_with("rule 9:"));
    assert!(invalid(&row().child(icon("star", "")).build()).starts_with("rule 9:"));
}

#[test]
fn validate_limits_image_sources() {
    view::validate(&column().child(image("assets/a.png", "A")).build()).unwrap();
    view::validate(
        &column()
            .child(image("https://example.com/a.png", "A"))
            .build(),
    )
    .unwrap();
    for bad in [
        "http://example.com/a.png",
        "/etc/a.png",
        "assets/../a.png",
        "file:///a.png",
        "a.png",
    ] {
        assert!(
            invalid(&column().child(image(bad, "A")).build()).starts_with("rule 10:"),
            "{bad}"
        );
    }
}

#[test]
fn empty_tree_is_invalid() {
    assert!(invalid(&ViewTree { nodes: vec![] }).starts_with("rule 1:"));
}
