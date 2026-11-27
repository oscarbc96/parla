//! The settings page's MCP section stays in step with the server: every tool is
//! listed in every language and the examples name no provider.

use std::path::{Path, PathBuf};

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/content/docs")
}

fn pages() -> Vec<(String, String)> {
    [
        ("settings.mdx", "## AI and MCP"),
        ("settings.es.mdx", "## IA y MCP"),
        ("settings.ca.mdx", "## IA i MCP"),
    ]
    .into_iter()
    .map(|(file, heading)| {
        let text = std::fs::read_to_string(docs_dir().join(file))
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let text = section(&text, heading)
            .unwrap_or_else(|| panic!("{file}: no `{heading}` section"))
            .to_string();
        (file.to_string(), text)
    })
    .collect()
}

/// The heading's text up to the next heading of the same level, so prose
/// elsewhere on the page cannot satisfy a check.
fn section<'a>(text: &'a str, heading: &str) -> Option<&'a str> {
    let level = heading.bytes().take_while(|b| *b == b'#').count();
    let marker = format!("\n{}", "#".repeat(level));
    let start = text.find(heading)?;
    let body = &text[start + heading.len()..];
    let end = body
        .match_indices(&marker)
        .find(|(i, _)| body[i + marker.len()..].starts_with(' '))
        .map_or(body.len(), |(i, _)| i);
    Some(&body[..end])
}

#[test]
fn a_section_stops_at_the_next_heading_of_the_same_level() {
    let text = "## MCP\n\nalpha\n\n### Detail\n\nbeta\n\n## Next\n\ngamma\n";
    assert_eq!(
        section(text, "## MCP").unwrap(),
        "\n\nalpha\n\n### Detail\n\nbeta\n"
    );
}

#[test]
fn docs_list_every_tool() {
    let tools = mcp::server::tools();
    assert!(tools.len() >= 12);
    for (file, text) in pages() {
        for t in &tools {
            assert!(
                text.contains(&format!("`{}`", t.name)),
                "{file} does not list the tool `{}`",
                t.name
            );
        }
    }
}

#[test]
fn docs_name_no_provider_in_examples() {
    let providers: Vec<String> =
        std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../providers"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_lowercase())
            .collect();
    for (file, text) in pages() {
        assert!(
            text.contains("<provider>/<id>"),
            "{file}: no neutral key example"
        );
        let lower = text.to_lowercase();
        for p in &providers {
            assert!(!lower.contains(p.as_str()), "{file} names provider {p}");
        }
    }
}

#[test]
fn docs_state_the_safety_rules() {
    for (file, text) in pages() {
        for needle in [
            "claude mcp add parla",
            "context_servers",
            "include_content",
            "confirm_writes",
        ] {
            assert!(text.contains(needle), "{file} is missing `{needle}`");
        }
    }
}
