//! `plugin.toml` parsing and validation. Fixtures are under
//! `tests/fixtures/manifests/`.

use plugin_host::manifest::{
    Activation, ArgPattern, Category, ErrorKind, LocalisedText, Manifest, ManifestError,
    ManifestSource, MediaKind, ReadArea, SettingKind, SettingValue, TransformKind, WriteArea,
    limits, parse,
};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/manifests/{name}.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn ok(src: &str) -> Manifest {
    parse(src, ManifestSource::Package)
        .unwrap_or_else(|e| panic!("expected a valid manifest, got:\n{e}"))
}

fn errors(src: &str) -> Vec<ManifestError> {
    errors_from(src, ManifestSource::Package)
}

fn errors_from(src: &str, source: ManifestSource) -> Vec<ManifestError> {
    match parse(src, source) {
        Ok(m) => panic!("expected errors, parsed {:?}", m.id),
        Err(e) => e.errors().to_vec(),
    }
}

fn has(errs: &[ManifestError], kind: ErrorKind, key: &str) -> bool {
    errs.iter().any(|e| e.kind == kind && e.key == key)
}

#[track_caller]
fn assert_has(errs: &[ManifestError], kind: ErrorKind, key: &str) {
    assert!(has(errs, kind, key), "no {kind:?} at `{key}` in {errs:#?}");
}

/// A minimal valid code-plugin manifest with `extra` appended.
fn minimal(id: &str, extra: &str) -> String {
    format!("id = \"{id}\"\nname = \"X\"\nversion = \"0.1.0\"\napi = \"0.1\"\n{extra}")
}

#[test]
fn full_example_parses() {
    let m = ok(&fixture("full"));
    assert_eq!(m.id.as_str(), "reminders-panel");
    assert_eq!(m.namespace, "reminders");
    assert_eq!(m.name, LocalisedText::Literal("Reminders panel".into()));
    assert_eq!(m.version.to_string(), "0.1.0");
    assert_eq!(m.api, (0, 1));
    assert_eq!(
        m.description,
        Some(LocalisedText::Literal(
            "Your reminders in a side panel.".into()
        ))
    );
    assert_eq!(m.authors, ["Ada Lindqvist"]);
    assert_eq!(
        m.repository.as_deref(),
        Some("https://github.com/ada/reminders-panel")
    );
    assert_eq!(m.license.as_deref(), Some("MIT"));
    assert_eq!(m.categories, [Category::Panels]);
    assert_eq!(
        m.min_app.as_ref().map(|v| v.to_string()).as_deref(),
        Some("0.4.0")
    );
    assert_eq!(m.limits.memory_mb, 16);
    assert!(!m.is_data());

    let caps = &m.capabilities;
    assert_eq!(
        caps.workspace_read,
        [ReadArea::Reminders, ReadArea::Messages]
    );
    assert_eq!(caps.workspace_write, [WriteArea::Reminders]);
    assert_eq!(caps.network.len(), 1);
    assert_eq!(caps.network[0].as_str(), "api.example.com");
    assert!(!caps.network[0].is_local());
    assert!(caps.storage && caps.windows && caps.panel);
    assert!(!caps.notifications && !caps.clipboard_write);
    assert_eq!(caps.exec.len(), 1);
    assert_eq!(caps.exec[0].program, "gh");
    assert_eq!(caps.exec[0].allow.len(), 3);
    assert_eq!(
        caps.exec[0].allow[2].0,
        [
            ArgPattern::Literal("api".into()),
            ArgPattern::Glob("repos/*".into())
        ]
    );

    let c = &m.contributions;
    assert_eq!(c.commands[0].id, "open");
    assert_eq!(c.commands[0].default_key.as_deref(), Some("cmd-shift-r"));
    assert_eq!(m.action_name(&c.commands[0].id), "reminders::open");
    assert_eq!(c.slash[0].name, "/later");
    assert_eq!(c.slash[0].usage.as_deref(), Some("/later [when]"));
    assert_eq!(c.message_actions[0].id, "remind");
    assert_eq!(c.composer_transforms[0].kind, TransformKind::Check);
    assert_eq!(c.renderers[0].patterns, ["tracker.example.com/*"]);
    assert_eq!(c.media_providers[0].kinds, [MediaKind::Gif]);
    assert_eq!(c.panels[0].icon, "bell");
    assert_eq!(c.mcp_tools[0].input_schema, "schemas/list_reminders.json");
    let hooks = c.hooks.as_ref().unwrap();
    assert_eq!(hooks.events.len(), 3);
    assert!(!hooks.notification_filter);

    assert_eq!(m.settings[0].key, "max_lines");
    assert_eq!(
        m.settings[0].kind,
        SettingKind::Integer {
            min: Some(5),
            max: Some(200)
        }
    );
    assert_eq!(m.settings[0].default, Some(SettingValue::Integer(20)));
    assert_eq!(m.themes.as_ref().unwrap().files, ["themes/solarised.json"]);
    assert_eq!(m.icon_packs[0].dir, "icons/outline");

    assert_eq!(m.activation.len(), 11);
    assert!(
        m.activation
            .contains(&Activation::Command("reminders::open".into()))
    );
    assert!(
        m.activation
            .contains(&Activation::Hook("channel_open".into()))
    );
}

#[test]
fn data_plugin_has_no_code_or_caps() {
    let m = ok(&fixture("data"));
    assert!(m.is_data());
    assert_eq!(m.capabilities, Default::default());
    assert!(m.contributions.is_empty());
    assert!(m.activation.is_empty());
    assert_eq!(m.name, LocalisedText::Key("plugin-name".into()));
    assert_eq!(m.categories, [Category::Themes, Category::IconPacks]);

    // Capabilities make it a code plugin, whatever else it ships.
    let with_caps = format!("{}\n[capabilities]\nstorage = true\n", fixture("data"));
    assert!(!ok(&with_caps).is_data());
}

#[test]
fn unknown_key_is_error() {
    let errs = errors(&fixture("unknown_key"));
    // Every unknown key is reported, each with its line, not just the first.
    assert_has(&errs, ErrorKind::UnknownKey, "capabilities.netwrok");
    assert_has(&errs, ErrorKind::UnknownKey, "commands[0].shortcut");
    let line = |key: &str| errs.iter().find(|e| e.key == key).unwrap().line;
    assert_eq!(line("capabilities.netwrok"), Some(8));
    assert_eq!(line("commands[0].shortcut"), Some(13));

    let errs = errors(&minimal("x-plugin", "colour = \"red\"\n"));
    assert_has(&errs, ErrorKind::UnknownKey, "colour");
}

#[test]
fn id_rules() {
    for good in ["abc", "a-b-c", "plugin-2", &"a".repeat(40)] {
        ok(&minimal(good, ""));
    }
    for bad in [
        "ab",
        &"a".repeat(41),
        "Upper",
        "under_score",
        "dot.ted",
        "spa ce",
        "",
    ] {
        let errs = errors(&minimal(bad, ""));
        assert_has(&errs, ErrorKind::InvalidId, "id");
        assert_eq!(errs.iter().find(|e| e.key == "id").unwrap().line, Some(1));
    }
}

#[test]
fn namespace_from_id() {
    let m = ok(&minimal("reminders-panel", ""));
    assert_eq!(m.namespace, "reminders_panel");
    assert_eq!(m.action_name("open"), "reminders_panel::open");

    let m = ok(&minimal("reminders-panel", "namespace = \"reminders\"\n"));
    assert_eq!(m.namespace, "reminders");

    let errs = errors(&minimal("reminders-panel", "namespace = \"Bad-Name\"\n"));
    assert_has(&errs, ErrorKind::InvalidNamespace, "namespace");
}

#[test]
fn activation_filled_from_contributions() {
    // With no `activation` key the host fills it as the packer would, so a
    // dev-mode folder behaves like a package.
    let src = minimal(
        "fill-me",
        r#"
[[commands]]
id = "open"
title = "Open"

[[slash]]
name = "/later"
description = "Later"

[[message_actions]]
id = "remind"
title = "Remind"

[[composer_transforms]]
id = "fix"
kind = "expand"
title = "Fix"

[[renderers]]
id = "links"
patterns = ["example.com/*"]

[[media_providers]]
id = "gifs"
title = "Gifs"
kinds = ["gif", "sticker"]

[[panels]]
id = "main"
title = "Main"
icon = "plugin"

[[mcp_tools]]
name = "list"
description = "List"
kind = "write"
input_schema = "schemas/list.json"

[hooks]
events = ["message"]
notification_filter = true
"#,
    );
    let m = ok(&src);
    assert_eq!(
        m.activation,
        [
            Activation::Command("fill_me::open".into()),
            Activation::Slash("/later".into()),
            Activation::MessageAction("remind".into()),
            Activation::Transform("fix".into()),
            Activation::Render("links".into()),
            Activation::Media("gifs".into()),
            Activation::Panel("main".into()),
            Activation::Mcp("list".into()),
            Activation::Hook("message".into()),
            Activation::Filter,
        ]
    );
    assert_eq!(m.implied_activation(), m.activation);
    assert_eq!(
        Activation::Command("fill_me::open".into()).to_string(),
        "command:fill_me::open"
    );
    assert_eq!(Activation::Filter.to_string(), "filter");
}

#[test]
fn contribution_without_event_is_error() {
    let errs = errors(&fixture("contribution_without_event"));
    assert_has(&errs, ErrorKind::ContributionWithoutEvent, "panels[0]");
    assert!(!has(
        &errs,
        ErrorKind::ContributionWithoutEvent,
        "commands[0]"
    ));

    // An event naming no contribution is an error too.
    let errs = errors(&minimal(
        "orphan-event",
        "activation = [\"panel:nowhere\"]\n",
    ));
    assert_has(&errs, ErrorKind::EventWithoutContribution, "activation[0]");

    let errs = errors(&minimal("bad-event", "activation = [\"whenever\"]\n"));
    assert_has(&errs, ErrorKind::InvalidActivation, "activation[0]");
}

#[test]
fn startup_activation_is_rejected() {
    let errs = errors(&fixture("startup_activation"));
    assert_has(&errs, ErrorKind::StartupActivation, "activation[0]");
    assert_eq!(errs[0].line, Some(5));
}

#[test]
fn network_host_rules() {
    let errs = errors(&fixture("network_hosts"));
    let at = |i: usize| format!("capabilities.network[{i}]");
    assert!(
        !errs.iter().any(|e| e.key == at(0) || e.key == at(1)),
        "{errs:#?}"
    );
    assert_has(&errs, ErrorKind::NetworkScheme, &at(2));
    assert_has(&errs, ErrorKind::NetworkScheme, &at(3));
    assert_has(&errs, ErrorKind::NetworkWildcard, &at(4));
    assert_has(&errs, ErrorKind::NetworkIpLiteral, &at(5));
    assert_has(&errs, ErrorKind::NetworkIpLiteral, &at(6));
    assert_has(&errs, ErrorKind::NetworkIpLiteral, &at(7));
    assert_has(&errs, ErrorKind::NetworkInvalidHost, &at(8));
    assert_has(&errs, ErrorKind::NetworkInvalidHost, &at(9));
    assert_has(&errs, ErrorKind::NetworkInvalidHost, &at(10));
    // Lines point at each entry.
    assert_eq!(errs.iter().find(|e| e.key == at(2)).unwrap().line, Some(10));

    let m = ok(&minimal(
        "net-ok",
        "[capabilities]\nnetwork = [\"api.example.com\", \"localhost\"]\n",
    ));
    assert!(m.capabilities.network[1].is_local());
    assert!(!m.capabilities.network[0].is_local());
}

#[test]
fn memory_mb_default_16_max_64() {
    assert_eq!(ok(&minimal("mem-default", "")).limits.memory_mb, 16);
    assert_eq!(
        ok(&minimal("mem-max", "[limits]\nmemory_mb = 64\n"))
            .limits
            .memory_mb,
        64
    );
    let errs = errors(&fixture("memory_too_high"));
    assert_has(&errs, ErrorKind::MemoryOutOfRange, "limits.memory_mb");
    assert_eq!(errs[0].line, Some(7));
    let errs = errors(&minimal("mem-zero", "[limits]\nmemory_mb = 0\n"));
    assert_has(&errs, ErrorKind::MemoryOutOfRange, "limits.memory_mb");
}

#[test]
fn unknown_area_is_error() {
    let errs = errors(&fixture("unknown_area"));
    assert_has(
        &errs,
        ErrorKind::UnknownArea,
        "capabilities.workspace_read[1]",
    );
    assert_has(
        &errs,
        ErrorKind::UnknownArea,
        "capabilities.workspace_read[2]",
    );
    assert_has(
        &errs,
        ErrorKind::UnknownArea,
        "capabilities.workspace_write[1]",
    );
    assert_eq!(errs.len(), 3, "{errs:#?}");
}

#[test]
fn areas_match_the_api_lists() {
    let read: Vec<&str> = ReadArea::ALL.iter().map(|a| a.as_str()).collect();
    assert_eq!(read, parla_plugin_api::READ_AREAS);
    let write: Vec<&str> = WriteArea::ALL.iter().map(|a| a.as_str()).collect();
    assert_eq!(write, parla_plugin_api::WRITE_AREAS);
}

#[test]
fn newer_minor_api_is_rejected() {
    let errs = errors(&fixture("newer_minor_api"));
    assert_has(&errs, ErrorKind::UnsupportedApi, "api");
    for bad in ["1", "0.1.0", "x.y", ""] {
        let src = format!("id = \"api-bad\"\nname = \"X\"\nversion = \"0.1.0\"\napi = \"{bad}\"\n");
        assert_has(&errors(&src), ErrorKind::InvalidApi, "api");
    }
    let src = "id = \"ver-bad\"\nname = \"X\"\nversion = \"one\"\napi = \"0.1\"\n";
    assert_has(&errors(src), ErrorKind::InvalidVersion, "version");
}

#[test]
fn renderer_patterns_max_16() {
    let errs = errors(&fixture("renderer_patterns_17"));
    assert_has(&errs, ErrorKind::TooManyPatterns, "renderers[0].patterns");

    let errs = errors(&minimal(
        "pattern-bad",
        "[[renderers]]\nid = \"r\"\npatterns = [\"*.example.com/x\", \"https://example.com/*\", \"\"]\n",
    ));
    assert_has(&errs, ErrorKind::InvalidPattern, "renderers[0].patterns[0]");
    assert_has(&errs, ErrorKind::InvalidPattern, "renderers[0].patterns[1]");
    assert_has(&errs, ErrorKind::InvalidPattern, "renderers[0].patterns[2]");
}

#[test]
fn exec_shape_rejects_path_program() {
    let errs = errors(&fixture("exec_path_program"));
    assert_has(
        &errs,
        ErrorKind::ExecProgramPath,
        "capabilities.exec[0].program",
    );
    // A flag is allowed only as a literal; an empty shape is no shape.
    assert_has(
        &errs,
        ErrorKind::InvalidArgShape,
        "capabilities.exec[1].allow[1]",
    );
    assert_has(
        &errs,
        ErrorKind::InvalidArgShape,
        "capabilities.exec[1].allow[2]",
    );
    assert!(!has(
        &errs,
        ErrorKind::InvalidArgShape,
        "capabilities.exec[1].allow[0]"
    ));
    for bad in ["..", ".", "bin\\gh", "", "./gh"] {
        let src = minimal(
            "exec-bad",
            &format!("[[capabilities.exec]]\nprogram = {bad:?}\nallow = [[\"x\"]]\n"),
        );
        assert_has(
            &errors(&src),
            ErrorKind::ExecProgramPath,
            "capabilities.exec[0].program",
        );
    }
}

#[test]
fn slash_name_must_start_with_slash() {
    let errs = errors(&fixture("slash_name"));
    assert_has(&errs, ErrorKind::InvalidSlashName, "slash[0].name");
    for bad in ["/", "/Later", "/two words"] {
        let src = minimal(
            "slash-bad",
            &format!("[[slash]]\nname = {bad:?}\ndescription = \"d\"\n"),
        );
        assert_has(&errors(&src), ErrorKind::InvalidSlashName, "slash[0].name");
    }
}

#[test]
fn settings_types() {
    let m = ok(&fixture("settings_types"));
    let kinds: Vec<&SettingKind> = m.settings.iter().map(|s| &s.kind).collect();
    assert_eq!(
        kinds,
        [
            &SettingKind::Boolean,
            &SettingKind::Integer {
                min: Some(5),
                max: Some(200)
            },
            &SettingKind::String,
            &SettingKind::Enum {
                options: vec!["small".into(), "large".into()]
            },
            &SettingKind::Secret,
        ]
    );
    assert_eq!(m.settings[0].default, Some(SettingValue::Boolean(true)));
    assert_eq!(
        m.settings[2].default,
        Some(SettingValue::String("hi".into()))
    );
    assert_eq!(
        m.settings[3].default,
        Some(SettingValue::String("small".into()))
    );
    assert_eq!(m.settings[4].default, None);
    assert_eq!(
        m.settings[1].title,
        LocalisedText::Key("setting-max-lines".into())
    );

    let errs = errors(&fixture("settings_bad"));
    assert_has(&errs, ErrorKind::SettingDefault, "settings[0].default");
    assert_has(&errs, ErrorKind::SettingDefault, "settings[1].default");
    assert_has(&errs, ErrorKind::SettingDefault, "settings[2].default");
    assert_has(&errs, ErrorKind::SettingDefault, "settings[3].default");
    assert_has(&errs, ErrorKind::SettingType, "settings[4].type");
}

#[test]
fn errors_are_collected_with_lines() {
    // Several independent rule breaks come back together.
    let src = "id = \"X\"\nname = \"X\"\nversion = \"0.1.0\"\napi = \"0.9\"\n\n[limits]\nmemory_mb = 100\n";
    let errs = errors(src);
    assert_has(&errs, ErrorKind::InvalidId, "id");
    assert_has(&errs, ErrorKind::UnsupportedApi, "api");
    assert_has(&errs, ErrorKind::MemoryOutOfRange, "limits.memory_mb");
    let lines: Vec<_> = errs.iter().map(|e| e.line).collect();
    assert_eq!(lines, [Some(1), Some(4), Some(7)]);

    // Syntax and type errors carry a line too.
    let errs = errors("id = \"x-y\"\nname = \n");
    assert_eq!(errs[0].kind, ErrorKind::Syntax);
    assert_eq!(errs[0].line, Some(2));
    let errs = errors(&minimal("type-bad", "authors = \"Ada\"\n"));
    assert_eq!(errs[0].kind, ErrorKind::Type);
    assert_eq!(errs[0].line, Some(5));
    let errs = errors("id = \"no-name\"\nversion = \"0.1.0\"\napi = \"0.1\"\n");
    assert_eq!(errs[0].kind, ErrorKind::Type);
}

#[test]
fn contribution_ids_and_paths() {
    let errs = errors(&minimal(
        "ids-bad",
        r#"
[[commands]]
id = "Open Now"
title = "Open"

[[commands]]
id = "dup"
title = "A"

[[commands]]
id = "dup"
title = "B"

[[mcp_tools]]
name = "t"
description = "d"
kind = "read"
input_schema = "../outside.json"

[themes]
files = ["/etc/passwd"]

[[icon_packs]]
name = "P"
dir = "icons/../../x"
"#,
    ));
    assert_has(&errs, ErrorKind::InvalidContributionId, "commands[0].id");
    assert_has(&errs, ErrorKind::DuplicateContribution, "commands[2].id");
    assert_has(&errs, ErrorKind::InvalidPath, "mcp_tools[0].input_schema");
    assert_has(&errs, ErrorKind::InvalidPath, "themes.files[0]");
    assert_has(&errs, ErrorKind::InvalidPath, "icon_packs[0].dir");
}

#[test]
fn dev_section_and_categories() {
    let src = minimal(
        "dev-plugin",
        "categories = [\"other\", \"media\"]\n[dev]\nwasm = \"target/wasm32-wasip2/release/x.wasm\"\n",
    );
    let m = parse(&src, ManifestSource::Dev).unwrap();
    assert_eq!(
        m.dev.as_ref().unwrap().wasm.as_deref(),
        Some("target/wasm32-wasip2/release/x.wasm")
    );
    assert_eq!(m.categories, [Category::Other, Category::Media]);
    // A package never carries `[dev]`.
    let errs = errors_from(&src, ManifestSource::Package);
    assert_has(&errs, ErrorKind::DevOnly, "dev");
    // Dev mode reads a plain manifest too.
    parse(&minimal("dev-plain", ""), ManifestSource::Dev).unwrap();

    let errs = errors(&minimal("cat-bad", "categories = [\"games\"]\n"));
    assert_eq!(errs[0].kind, ErrorKind::Type);
}

#[test]
fn source_size_is_capped() {
    let pad = format!("# {}\n", "x".repeat(limits::MAX_SOURCE_BYTES));
    let errs = errors(&minimal("big-file", &pad));
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, ErrorKind::TooLarge);
    // Just under the cap still parses.
    let room = limits::MAX_SOURCE_BYTES - minimal("big-file", "").len() - 3;
    ok(&minimal("big-file", &format!("# {}\n", "x".repeat(room))));
}

#[test]
fn numeric_and_local_hosts_are_rejected() {
    let at = |i: usize| format!("capabilities.network[{i}]");
    let bad = [
        "127.1",
        "0x7f.1",
        "10.0.0.0x1",
        "1.2.3",
        "foo.localhost",
        "a.b.localhost",
        "api.0x10",
        "example.123",
    ];
    let list: Vec<String> = bad.iter().map(|h| format!("{h:?}")).collect();
    let errs = errors(&minimal(
        "net-numeric",
        &format!("[capabilities]\nnetwork = [{}]\n", list.join(", ")),
    ));
    for (i, host) in bad.iter().enumerate() {
        assert!(
            errs.iter().any(|e| e.key == at(i)),
            "{host} accepted: {errs:#?}"
        );
    }
    ok(&minimal(
        "net-fine",
        "[capabilities]\nnetwork = [\"api.example.com\", \"x1.example.io\", \"cafe.dev\"]\n",
    ));

    let errs = errors(&minimal(
        "pattern-num",
        "[[renderers]]\nid = \"r\"\npatterns = [\"127.1/*\", \"app.localhost/*\", \"10.0.0.1/x\"]\n",
    ));
    for i in 0..3 {
        assert_has(
            &errs,
            ErrorKind::InvalidPattern,
            &format!("renderers[0].patterns[{i}]"),
        );
    }
}

#[test]
fn counts_and_lengths_are_capped() {
    let long = "x".repeat(limits::MAX_NAME_CHARS + 1);
    let errs = errors(&format!(
        "id = \"long-name\"\nname = {long:?}\nversion = \"0.1.0\"\napi = \"0.1\"\n"
    ));
    assert_has(&errs, ErrorKind::TooLong, "name");

    let long = "x".repeat(limits::MAX_DESCRIPTION_CHARS + 1);
    assert_has(
        &errors(&minimal("long-desc", &format!("description = {long:?}\n"))),
        ErrorKind::TooLong,
        "description",
    );

    let long = "x".repeat(limits::MAX_AUTHOR_CHARS + 1);
    assert_has(
        &errors(&minimal("long-author", &format!("authors = [{long:?}]\n"))),
        ErrorKind::TooLong,
        "authors[0]",
    );
    let many: Vec<String> = (0..=limits::MAX_AUTHORS)
        .map(|i| format!("\"a{i}\""))
        .collect();
    assert_has(
        &errors(&minimal(
            "many-authors",
            &format!("authors = [{}]\n", many.join(",")),
        )),
        ErrorKind::TooMany,
        "authors",
    );

    let long = "x".repeat(limits::MAX_TITLE_CHARS + 1);
    assert_has(
        &errors(&minimal(
            "long-title",
            &format!("[[commands]]\nid = \"open\"\ntitle = {long:?}\n"),
        )),
        ErrorKind::TooLong,
        "commands[0].title",
    );

    let hosts: Vec<String> = (0..=limits::MAX_NETWORK_HOSTS)
        .map(|i| format!("\"h{i}.example.com\""))
        .collect();
    assert_has(
        &errors(&minimal(
            "many-hosts",
            &format!("[capabilities]\nnetwork = [{}]\n", hosts.join(",")),
        )),
        ErrorKind::TooMany,
        "capabilities.network",
    );

    let exec = "[[capabilities.exec]]\nprogram = \"gh\"\nallow = [[\"pr\"]]\n"
        .repeat(limits::MAX_EXEC_RULES + 1);
    assert_has(
        &errors(&minimal("many-exec", &exec)),
        ErrorKind::TooMany,
        "capabilities.exec",
    );

    let shapes: Vec<&str> = vec!["[\"pr\"]"; limits::MAX_EXEC_SHAPES + 1];
    assert_has(
        &errors(&minimal(
            "many-shapes",
            &format!(
                "[[capabilities.exec]]\nprogram = \"gh\"\nallow = [{}]\n",
                shapes.join(",")
            ),
        )),
        ErrorKind::TooMany,
        "capabilities.exec[0].allow",
    );
    let args: Vec<&str> = vec!["\"x\""; limits::MAX_SHAPE_ARGS + 1];
    assert_has(
        &errors(&minimal(
            "many-args",
            &format!(
                "[[capabilities.exec]]\nprogram = \"gh\"\nallow = [[{}]]\n",
                args.join(",")
            ),
        )),
        ErrorKind::TooMany,
        "capabilities.exec[0].allow[0]",
    );

    let settings: String = (0..=limits::MAX_SETTINGS)
        .map(|i| format!("[[settings]]\nkey = \"k{i}\"\ntype = \"boolean\"\ntitle = \"K\"\n"))
        .collect();
    assert_has(
        &errors(&minimal("many-settings", &settings)),
        ErrorKind::TooMany,
        "settings",
    );

    let commands: String = (0..=limits::MAX_CONTRIBUTIONS)
        .map(|i| format!("[[commands]]\nid = \"c{i}\"\ntitle = \"C\"\n"))
        .collect();
    assert_has(
        &errors(&minimal("many-commands", &commands)),
        ErrorKind::TooMany,
        "commands",
    );

    // At the caps everything still parses.
    let commands: String = (0..limits::MAX_CONTRIBUTIONS)
        .map(|i| format!("[[commands]]\nid = \"c{i}\"\ntitle = \"C\"\n"))
        .collect();
    ok(&minimal("max-commands", &commands));
}

#[test]
fn dashes_in_ids() {
    for bad in ["-abc", "abc-", "---", "-a-"] {
        assert_has(&errors(&minimal(bad, "")), ErrorKind::InvalidId, "id");
    }
    let errs = errors(&minimal(
        "dash-contrib",
        "[[commands]]\nid = \"-open\"\ntitle = \"O\"\n[[panels]]\nid = \"main-\"\ntitle = \"M\"\nicon = \"plugin\"\n",
    ));
    assert_has(&errs, ErrorKind::InvalidContributionId, "commands[0].id");
    assert_has(&errs, ErrorKind::InvalidContributionId, "panels[0].id");
}

#[test]
fn default_key_icon_and_repository() {
    for good in ["cmd-shift-r", "ctrl-alt-k", "secondary-f5", "alt-enter"] {
        ok(&minimal(
            "key-ok",
            &format!("[[commands]]\nid = \"o\"\ntitle = \"O\"\ndefault_key = {good:?}\n"),
        ));
    }
    for bad in [
        "r",
        "shift-r",
        "cmd-cmd-r",
        "cmd-",
        "cmd-shift-r cmd-k",
        "CMD-R",
        "hyper-r",
        "",
    ] {
        let src = minimal(
            "key-bad",
            &format!("[[commands]]\nid = \"o\"\ntitle = \"O\"\ndefault_key = {bad:?}\n"),
        );
        assert_has(
            &errors(&src),
            ErrorKind::InvalidKey,
            "commands[0].default_key",
        );
    }

    for bad in ["Bell", "", "../x", "a b"] {
        let src = minimal(
            "icon-bad",
            &format!("[[panels]]\nid = \"p\"\ntitle = \"P\"\nicon = {bad:?}\n"),
        );
        assert_has(&errors(&src), ErrorKind::InvalidIcon, "panels[0].icon");
    }

    ok(&minimal(
        "repo-ok",
        "repository = \"https://example.com/ada/x\"\n",
    ));
    for bad in [
        "http://example.com/x",
        "git@example.com:x",
        "https://",
        "https://127.1/x",
        "example.com/x",
    ] {
        let src = minimal("repo-bad", &format!("repository = {bad:?}\n"));
        assert_has(&errors(&src), ErrorKind::InvalidRepository, "repository");
    }
}

#[test]
fn fluent_keys_are_checked() {
    ok(&minimal(
        "key-ok",
        "description = { key = \"plugin-description_2\" }\n",
    ));
    for bad in ["", "2nd", "has space", "dot.ted", "-lead"] {
        let src = minimal("key-bad", &format!("description = {{ key = {bad:?} }}\n"));
        assert_has(&errors(&src), ErrorKind::InvalidFluentKey, "description");
    }
    let errs = errors(&minimal(
        "key-bad",
        "[[commands]]\nid = \"o\"\ntitle = { key = \"a b\" }\n",
    ));
    assert_has(&errs, ErrorKind::InvalidFluentKey, "commands[0].title");
}

#[test]
fn duplicate_enum_options_and_hook_events() {
    let errs = errors(&minimal(
        "dups",
        "[hooks]\nevents = [\"message\", \"reaction\", \"message\"]\n\n[[settings]]\nkey = \"s\"\ntype = \"enum\"\noptions = [\"a\", \"b\", \"a\"]\ntitle = \"S\"\n",
    ));
    assert_has(&errs, ErrorKind::DuplicateContribution, "hooks.events[2]");
    assert_has(
        &errs,
        ErrorKind::DuplicateContribution,
        "settings[0].options[2]",
    );
}

#[test]
fn exec_never_runs_a_shell_command() {
    for (program, flag) in [
        ("sh", "-c"),
        ("bash", "-c"),
        ("zsh", "-c"),
        ("cmd", "/c"),
        ("cmd.exe", "/C"),
        ("powershell", "-c"),
        ("pwsh", "-Command"),
    ] {
        let src = minimal(
            "shell-exec",
            &format!("[[capabilities.exec]]\nprogram = {program:?}\nallow = [[{flag:?}, \"*\"]]\n"),
        );
        assert_has(
            &errors(&src),
            ErrorKind::ExecShell,
            "capabilities.exec[0].allow[0]",
        );
    }
    // A shell running a fixed script with no -c is a plain shape.
    ok(&minimal(
        "shell-script",
        "[[capabilities.exec]]\nprogram = \"sh\"\nallow = [[\"tools/run.sh\"]]\n",
    ));
    // A `*` matches within one argument: a pattern with whitespace would
    // stand for a command line, not an argument.
    let errs = errors(&minimal(
        "space-glob",
        "[[capabilities.exec]]\nprogram = \"gh\"\nallow = [[\"api\", \"repos/* --x\"]]\n",
    ));
    assert_has(
        &errs,
        ErrorKind::InvalidArgShape,
        "capabilities.exec[0].allow[0]",
    );
}

#[test]
fn memory_constants_are_the_design_values() {
    assert_eq!(plugin_host::manifest::DEFAULT_MEMORY_MB, 16);
    assert_eq!(plugin_host::manifest::MAX_MEMORY_MB, 64);
}

#[test]
fn theme_files_and_icon_packs_stay_in_their_folders() {
    for (themes, dir) in [
        ("assets/t.json", "assets/icons"),
        ("themes/sub/t.json", "icons"),
        ("themes/t.toml", "icons/a/b"),
        ("t.json", "themes/pack"),
    ] {
        let errs = errors(&minimal(
            "folder-rules",
            &format!(
                "[themes]\nfiles = [\"{themes}\"]\n[[icon_packs]]\nname = \"P\"\ndir = \"{dir}\"\n"
            ),
        ));
        assert_has(&errs, ErrorKind::InvalidPath, "themes.files[0]");
        assert_has(&errs, ErrorKind::InvalidPath, "icon_packs[0].dir");
    }
}
