//! The WIT package parses, has the expected shape, and stays provider-neutral.

use std::path::Path;

use parla_plugin_api::{API_VERSION, READ_AREAS, WIT_DIR, WRITE_AREAS, supports};
use wit_parser::{
    Function, InterfaceId, PackageId, Resolve, Type, TypeDefKind, TypeId, WorldId, WorldItem,
    WorldKey,
};

/// Optional extension points, each its own interface.
const EXTENSION_POINTS: &[&str] = &[
    "deactivation",
    "migration",
    "commands",
    "slash-commands",
    "message-actions",
    "event-hooks",
    "notification-filter",
    "composer-transforms",
    "renderer",
    "media-provider",
    "views",
    "settings-listener",
    "mcp-tools",
];

fn load() -> (Resolve, PackageId) {
    let mut resolve = Resolve::default();
    let (pkg, _) = resolve.push_dir(WIT_DIR).expect("the WIT package resolves");
    (resolve, pkg)
}

fn plugin_world(resolve: &Resolve, pkg: PackageId) -> WorldId {
    resolve
        .select_world(&[pkg], Some("plugin"))
        .expect("world `plugin` exists")
}

fn iface(resolve: &Resolve, pkg: PackageId, name: &str) -> InterfaceId {
    *resolve.packages[pkg]
        .interfaces
        .get(name)
        .unwrap_or_else(|| panic!("interface `{name}` exists"))
}

fn func<'a>(resolve: &'a Resolve, pkg: PackageId, i: &str, f: &str) -> &'a Function {
    resolve.interfaces[iface(resolve, pkg, i)]
        .functions
        .get(f)
        .unwrap_or_else(|| panic!("`{i}.{f}` exists"))
}

fn param_names(f: &Function) -> Vec<&str> {
    f.params.iter().map(|p| p.name.as_str()).collect()
}

fn type_in(resolve: &Resolve, pkg: PackageId, i: &str, t: &str) -> TypeId {
    resolve.interfaces[iface(resolve, pkg, i)].types[t]
}

/// Follows `use` aliases to the defining type.
fn resolved(resolve: &Resolve, mut id: TypeId) -> TypeId {
    while let TypeDefKind::Type(Type::Id(next)) = resolve.types[id].kind {
        id = next;
    }
    id
}

fn field_names(resolve: &Resolve, id: TypeId) -> Vec<String> {
    match &resolve.types[resolved(resolve, id)].kind {
        TypeDefKind::Record(r) => r.fields.iter().map(|f| f.name.clone()).collect(),
        other => panic!("expected a record, got {other:?}"),
    }
}

fn case_names(resolve: &Resolve, id: TypeId) -> Vec<String> {
    match &resolve.types[resolved(resolve, id)].kind {
        TypeDefKind::Variant(v) => v.cases.iter().map(|c| c.name.clone()).collect(),
        other => panic!("expected a variant, got {other:?}"),
    }
}

/// Bare interface names (`host`, `ui`, ...) of a world's imports or exports.
fn interface_names(resolve: &Resolve, items: &[(&WorldKey, &WorldItem)]) -> Vec<String> {
    items
        .iter()
        .filter_map(|(_, item)| match item {
            WorldItem::Interface { id, .. } => resolve.interfaces[*id].name.clone(),
            _ => None,
        })
        .collect()
}

/// Resolves a guest package that builds a world on top of `plugin`.
fn guest_world(body: &str) -> Result<(), String> {
    let (mut resolve, _) = load();
    let src = format!("package example:guest;\nworld guest {{\n{body}\n}}\n");
    let pkg = resolve
        .push_str("guest.wit", &src)
        .map_err(|e| format!("{e:#}"))?;
    resolve
        .select_world(&[pkg], Some("guest"))
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

fn wit_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(Path::new(WIT_DIR)).expect("WIT dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|e| e == "wit") {
            let text = std::fs::read_to_string(&path).expect("read WIT");
            out.push((path.display().to_string(), text));
        }
    }
    assert!(
        out.len() >= 10,
        "expected the WIT files, found {}",
        out.len()
    );
    out
}

#[test]
fn wit_package_parses() {
    let (resolve, pkg) = load();
    let name = &resolve.packages[pkg].name;
    assert_eq!(name.namespace, "parla");
    assert_eq!(name.name, "plugin");
    let version = name.version.as_ref().expect("versioned package");
    assert_eq!(version.to_string(), "0.1.0");
    let (major, minor) = API_VERSION;
    assert_eq!((version.major, version.minor), (major.into(), minor.into()));
}

#[test]
fn world_plugin_exports_only_lifecycle_activate() {
    let (resolve, pkg) = load();
    let world = &resolve.worlds[plugin_world(&resolve, pkg)];
    let exports: Vec<_> = world.exports.iter().collect();
    assert_eq!(interface_names(&resolve, &exports), ["lifecycle"]);
    let lifecycle = &resolve.interfaces[iface(&resolve, pkg, "lifecycle")];
    let functions: Vec<_> = lifecycle.functions.keys().map(String::as_str).collect();
    assert_eq!(functions, ["activate"]);
}

#[test]
fn a_world_with_only_lifecycle_is_valid() {
    guest_world("include parla:plugin/plugin@0.1.0;").expect("lifecycle-only guest world");
}

#[test]
fn every_extension_point_is_its_own_optional_export() {
    for name in EXTENSION_POINTS {
        guest_world(&format!(
            "include parla:plugin/plugin@0.1.0;\nexport parla:plugin/{name}@0.1.0;"
        ))
        .unwrap_or_else(|e| panic!("guest world exporting `{name}`: {e}"));
    }
}

#[test]
fn every_host_interface_is_imported() {
    let (resolve, pkg) = load();
    let world = &resolve.worlds[plugin_world(&resolve, pkg)];
    let imports: Vec<_> = world.imports.iter().collect();
    let names = interface_names(&resolve, &imports);
    for host in [
        "host",
        "workspace",
        "http",
        "storage",
        "secrets",
        "settings",
        "ui",
        "composer",
        "exec",
    ] {
        assert!(
            names.iter().any(|n| n == host),
            "import `{host}` missing: {names:?}"
        );
    }
}

#[test]
fn error_variant_matches_design() {
    let (resolve, pkg) = load();
    let error = type_in(&resolve, pkg, "types", "error");
    let TypeDefKind::Variant(variant) = &resolve.types[error].kind else {
        panic!("`error` is a variant");
    };
    let cases: Vec<(&str, bool)> = variant
        .cases
        .iter()
        .map(|c| (c.name.as_str(), c.ty.is_some()))
        .collect();
    assert_eq!(
        cases,
        [
            ("denied", true),
            ("unsupported", false),
            ("not-found", false),
            ("limit", true),
            ("invalid", true),
            ("offline", false),
            ("cancelled", false),
            ("failed", true),
        ]
    );
    for case in &variant.cases {
        if matches!(case.name.as_str(), "limit" | "invalid" | "failed") {
            assert_eq!(
                case.ty,
                Some(Type::String),
                "`{}` carries a string",
                case.name
            );
        }
    }
}

#[test]
fn render_and_run_slash_get_a_host_view_id() {
    let (resolve, pkg) = load();
    for (i, f) in [("renderer", "render"), ("slash-commands", "run-slash")] {
        let function = func(&resolve, pkg, i, f);
        assert_eq!(
            param_names(function)[0],
            "view",
            "`{f}` takes the view id first"
        );
        let Type::Id(id) = function.params[0].ty else {
            panic!("`{f}` view param is a named type");
        };
        assert_eq!(resolve.types[id].name.as_deref(), Some("view-id"));
    }
}

#[test]
fn every_list_call_is_paged() {
    let (resolve, pkg) = load();
    for f in [
        "list-conversations",
        "history",
        "replies",
        "search-messages",
        "search-files",
        "list-reminders",
        "list-scheduled",
        "list-saved",
        "list-pins",
    ] {
        let params = param_names(func(&resolve, pkg, "workspace", f));
        assert!(
            params.contains(&"cursor") && params.contains(&"limit"),
            "`{f}` takes cursor and limit: {params:?}"
        );
    }
    let docs = &resolve.interfaces[iface(&resolve, pkg, "workspace")].functions["list-workspaces"]
        .docs
        .contents;
    assert!(
        docs.as_deref().unwrap_or("").contains("at most 100"),
        "list-workspaces documents its cap"
    );
}

#[test]
fn picker_values_carry_the_workspace() {
    let (resolve, pkg) = load();
    let value = resolved(&resolve, type_in(&resolve, pkg, "view", "field-value"));
    let TypeDefKind::Variant(v) = &resolve.types[value].kind else {
        panic!("field-value is a variant");
    };
    for picked in ["conversation", "user"] {
        let case = v.cases.iter().find(|c| c.name == picked).expect("case");
        let Some(Type::Id(id)) = case.ty else {
            panic!("`{picked}` carries a record");
        };
        assert!(field_names(&resolve, id).contains(&"workspace".to_string()));
    }
}

#[test]
fn content_mentions_are_structured_spans() {
    let (resolve, pkg) = load();
    let inline = type_in(&resolve, pkg, "types", "inline");
    assert_eq!(
        case_names(&resolve, inline),
        ["text", "mention-user", "mention-conversation", "link"]
    );
    for record in ["content", "message"] {
        let fields = field_names(&resolve, type_in(&resolve, pkg, "types", record));
        assert!(
            fields.contains(&"spans".to_string()),
            "{record}: {fields:?}"
        );
        assert!(
            !fields.contains(&"text".to_string()),
            "{record}: {fields:?}"
        );
    }
    for (path, text) in wit_files() {
        assert!(
            !text.contains("<@"),
            "{path} uses a provider mention format"
        );
    }
}

#[test]
fn view_tree_rules_are_documented() {
    let (resolve, pkg) = load();
    let docs = resolve.interfaces[iface(&resolve, pkg, "view")]
        .docs
        .contents
        .clone()
        .unwrap_or_default();
    for rule in [
        "node 0 is the only node without a parent",
        "less than its own index",
        "ids are unique",
        "need\n an id",
        "have children",
        "one child per label",
        "exactly 2 children",
        "`invalid`",
    ] {
        let flat = docs.split_whitespace().collect::<Vec<_>>().join(" ");
        let want = rule.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains(&want), "view rule missing: {rule}");
    }
}

#[test]
fn table_loads_more_and_tree_entries_expand() {
    let (resolve, pkg) = load();
    let table = type_in(&resolve, pkg, "view", "table-block");
    assert!(field_names(&resolve, table).contains(&"load-more".to_string()));
    let event = type_in(&resolve, pkg, "view", "event");
    let cases = case_names(&resolve, event);
    assert!(cases.contains(&"expand".to_string()), "{cases:?}");
}

#[test]
fn migrate_takes_a_version_and_media_items_have_a_mime_type() {
    let (resolve, pkg) = load();
    let migrate = func(&resolve, pkg, "migration", "migrate");
    let Type::Id(id) = migrate.params[0].ty else {
        panic!("migrate takes a named type");
    };
    assert_eq!(
        resolve.types[resolved(&resolve, id)].name.as_deref(),
        Some("version")
    );
    let item = type_in(&resolve, pkg, "media-provider", "media-item");
    assert!(field_names(&resolve, item).contains(&"mime-type".to_string()));
    let open = param_names(func(&resolve, pkg, "ui", "open-conversation"));
    assert_eq!(open, ["ws", "conversation", "message"]);
}

#[test]
fn wit_areas_come_from_the_one_list() {
    let known: Vec<&str> = READ_AREAS.iter().chain(WRITE_AREAS).copied().collect();
    let mut seen = 0;
    for (path, text) in wit_files() {
        for piece in text.split('`').skip(1).step_by(2) {
            let is_area = piece.split_once(':').is_some_and(|(a, b)| {
                !a.is_empty()
                    && a.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                    && matches!(b, "read" | "write" | "edit")
            });
            if is_area {
                assert!(known.contains(&piece), "{path}: unknown area `{piece}`");
                seen += 1;
            }
        }
    }
    assert!(seen > 20, "WIT docs name the areas ({seen})");
}

/// Provider names and provider-specific scope strings, reversed so that this
/// core test does not spell them out itself (CLAUDE.md, core neutrality).
const FORBIDDEN_REVERSED: &[&str] = &[
    "kcals",
    "xox",
    "drocsid",
    "tsomrettam",
    "piluz",
    "smaetsm",
    "tfosorcim",
    "tahc elgoog",
    "tahcelgoog",
    "etirw:tahc",
    "yrotsih:mi",
    "daer:mi",
    ":mipm",
    "yrotsih:spuorg",
    "daer:spuorg",
    "daer:maet",
    ":spuorgresu",
    "daer:ijome",
    "yrotsih:slennahc",
    "tsil:slennahc",
    "daer:sresu",
];

#[test]
fn wit_names_no_provider() {
    let forbidden: Vec<String> = FORBIDDEN_REVERSED
        .iter()
        .map(|t| t.chars().rev().collect())
        .collect();
    let mut names: Vec<&str> = READ_AREAS.to_vec();
    names.extend(WRITE_AREAS);
    for (path, text) in wit_files() {
        let text = text.to_lowercase();
        for term in &forbidden {
            assert!(
                !text.contains(term.as_str()),
                "{path} names a provider or a provider scope"
            );
        }
    }
    for area in names {
        for term in &forbidden {
            assert!(
                !area.contains(term.as_str()),
                "area `{area}` is a provider scope"
            );
        }
    }
}

#[test]
fn supports_only_the_exact_minor_while_major_is_zero() {
    let (major, minor) = API_VERSION;
    assert_eq!(major, 0, "update this test when 1.0 ships");
    assert!(supports(API_VERSION));
    assert!(!supports((major, minor - 1)));
    assert!(!supports((major, minor + 1)));
    assert!(!supports((major + 1, 0)));
    assert!(!supports((major + 1, minor)));
}
