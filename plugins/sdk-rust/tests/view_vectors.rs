//! The shared view-tree vectors (`crates/parla_plugin_api/tests/fixtures/
//! view-trees`, format in its README): `validate` gives each file's verdict
//! and rule, and `size` its measured size.

use std::path::PathBuf;

use parla_plugin_sdk::Error;
use parla_plugin_sdk::view::*;
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/parla_plugin_api/tests/fixtures/view-trees")
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
fn os(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}
fn tone(v: &Value) -> Tone {
    match v.as_str().unwrap_or("accent") {
        "accent" => Tone::Accent,
        "success" => Tone::Success,
        "warning" => Tone::Warning,
        "danger" => Tone::Danger,
        "info" => Tone::Info,
        "muted" => Tone::Muted,
        t => panic!("tone {t}"),
    }
}
fn text_input(v: &Value) -> TextInput {
    TextInput {
        label: s(v, "label"),
        value: s(v, "value"),
        placeholder: os(v, "placeholder"),
    }
}
fn choices(v: &Value) -> Vec<Choice> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|c| Choice {
            value: s(c, "value"),
            label: s(c, "label"),
        })
        .collect()
}
fn select_input(v: &Value) -> SelectInput {
    SelectInput {
        label: s(v, "label"),
        options: choices(&v["options"]),
        value: os(v, "value"),
    }
}
fn toggle(v: &Value) -> ToggleInput {
    ToggleInput {
        label: s(v, "label"),
        checked: b(v, "checked"),
    }
}

fn kind(v: &Value) -> NodeKind {
    if let Some(name) = v.as_str() {
        return match name {
            "column" => NodeKind::Column,
            "row" => NodeKind::Row,
            "card" => NodeKind::Card,
            "divider" => NodeKind::Divider,
            "spacer" => NodeKind::Spacer,
            "split" => NodeKind::Split,
            "form" => NodeKind::Form,
            "spinner" => NodeKind::Spinner,
            "skeleton" => NodeKind::Skeleton,
            k => panic!("unknown unit kind {k}"),
        };
    }
    let (name, p) = v.as_object().unwrap().iter().next().unwrap();
    match name.as_str() {
        "grid" => NodeKind::Grid(p.as_u64().unwrap() as u8),
        "tabs" => NodeKind::Tabs(strs(p)),
        "heading" => NodeKind::Heading(p.as_str().unwrap().into()),
        "label" => NodeKind::Label(p.as_str().unwrap().into()),
        "caption" => NodeKind::Caption(p.as_str().unwrap().into()),
        "chip" => NodeKind::Chip(p.as_str().unwrap().into()),
        "text" => NodeKind::Text(
            p.as_array()
                .unwrap()
                .iter()
                .map(|sp| Span {
                    text: s(sp, "text"),
                    bold: b(sp, "bold"),
                    italic: b(sp, "italic"),
                    code: b(sp, "code"),
                    link: os(sp, "link"),
                })
                .collect(),
        ),
        "table" => NodeKind::Table(TableBlock {
            columns: strs(&p["columns"]),
            rows: p["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .map(strs)
                .collect(),
            selectable: b(p, "selectable"),
            load_more: b(p, "load-more"),
        }),
        "list" => NodeKind::List(ListBlock {
            selectable: b(p, "selectable"),
            load_more: b(p, "load-more"),
        }),
        "badge" => NodeKind::Badge(BadgeBlock {
            text: s(p, "text"),
            tone: tone(&p["tone"]),
        }),
        "image" => NodeKind::Image(ImageBlock {
            src: s(p, "src"),
            alt: s(p, "alt"),
            width: None,
            height: None,
        }),
        "icon" => NodeKind::Icon(IconBlock {
            name: s(p, "name"),
            label: s(p, "label"),
            tone: None,
        }),
        "input" => NodeKind::Input(text_input(p)),
        "textarea" => NodeKind::Textarea(text_input(p)),
        "select" => NodeKind::Select(select_input(p)),
        "segmented" => NodeKind::Segmented(select_input(p)),
        "checkbox" => NodeKind::Checkbox(toggle(p)),
        "switch" => NodeKind::Switch(toggle(p)),
        "button" => NodeKind::Button(ButtonBlock {
            label: s(p, "label"),
            look: match p.get("look").and_then(Value::as_str).unwrap_or("default") {
                "primary" => ButtonLook::Primary,
                "ghost" => ButtonLook::Ghost,
                "danger" => ButtonLook::Danger,
                _ => ButtonLook::Default,
            },
            disabled: b(p, "disabled"),
        }),
        "conversation-picker" => NodeKind::ConversationPicker(Picker {
            label: s(p, "label"),
        }),
        "user-picker" => NodeKind::UserPicker(Picker {
            label: s(p, "label"),
        }),
        "callout" => NodeKind::Callout(CalloutBlock {
            tone: tone(&p["tone"]),
            title: os(p, "title"),
            text: s(p, "text"),
        }),
        "empty" => NodeKind::Empty(EmptyBlock {
            title: s(p, "title"),
            text: os(p, "text"),
        }),
        k => panic!("kind {k} not in the vector reader yet"),
    }
}

fn tree(v: &Value) -> ViewTree {
    let nodes = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| Node {
            id: os(n, "id"),
            parent: n["parent"].as_u64().map(|p| p as u32),
            kind: kind(&n["kind"]),
        })
        .collect();
    ViewTree { nodes }
}

#[test]
fn shared_view_tree_vectors() {
    let mut files: Vec<_> = std::fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    assert!(files.len() >= 20, "vectors missing in {}", dir().display());
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let t = tree(&v);
        let got = validate(&t);
        match (v["verdict"].as_str().unwrap(), &got) {
            ("ok", Ok(())) => {
                assert_eq!(
                    size(&t),
                    v["size"].as_u64().unwrap() as usize,
                    "{name}: size"
                );
            }
            ("invalid", Err(Error::Invalid(why))) | ("limit", Err(Error::Limit(why))) => {
                let rule = v["rule"].as_u64().unwrap();
                assert!(
                    why.starts_with(&format!("rule {rule}:")),
                    "{name}: wanted rule {rule}, got {why}"
                );
            }
            (want, got) => panic!("{name}: wanted {want}, got {got:?}"),
        }
    }
}
