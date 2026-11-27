//! Guard: no label the UI derives from a realistic workspace with unresolved
//! members is a raw provider id or raw markup. Covers the sidebar (section
//! headers, DM and group labels), the conversation header, and every author,
//! replier and message body of the timeline and thread.

use std::collections::HashSet;

use components::{SidebarRow, author_name, build_rows, header_info, user_name};
use settings::SidebarSettings;
use ui_testing::scenario;
use ui_testing::source::ScenarioSession;

/// A provider user, bot or workspace id: a `U`, `B` or `W` followed by eight
/// or more upper-case letters or digits (the whole label).
fn looks_like_raw_id(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some('U' | 'B' | 'W'))
        && s.len() >= 9
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn bad(s: &str) -> bool {
    looks_like_raw_id(s.trim()) || ["<@", "<#", "<http", "<!"].iter().any(|m| s.contains(m))
}

fn en() -> i18n::Localizer {
    let pref = i18n::LocalePref::Code("en".parse().unwrap());
    i18n::Localizer::new(
        i18n::chain(&pref, &[], i18n::LocaleSet::embedded()),
        &[components::catalog()],
    )
    .unwrap()
}

#[test]
fn the_matcher_matches_ids_and_not_names() {
    assert!(looks_like_raw_id("U0AB12CD34E"));
    assert!(looks_like_raw_id("W0123456789"));
    assert!(!looks_like_raw_id("Unknown"));
    assert!(!looks_like_raw_id("Ana Reyes"));
    assert!(!looks_like_raw_id("U1"));
    assert!(bad("see <@U1> and <#C1>"));
    assert!(bad("<https://example.com|x>"));
}

#[test]
fn no_scenario_shows_a_raw_id_or_raw_markup() {
    let l = en();
    let mut seen_unknown = false;
    for s in scenario::all().into_iter().filter(|s| !s.sign_in) {
        let session = ScenarioSession::new(&s);
        let state = session.state();
        let mut labels: Vec<String> = Vec::new();
        let rows = build_rows(
            state,
            s.open.as_ref(),
            &SidebarSettings::default(),
            &HashSet::new(),
            &l,
        );
        for r in rows {
            match r {
                SidebarRow::Header { label: h, .. } => labels.push(h.to_string()),
                SidebarRow::Conversation { label, .. } | SidebarRow::ShowMore { label, .. } => {
                    labels.push(label.to_string())
                }
            }
        }
        if let Some(c) = s.open.as_ref().and_then(|o| state.conversations.get(o)) {
            labels.push(header_info(state, c, &l).title);
        }
        let parser = components::TimelineSource::content(&session);
        for w in state.windows.values() {
            for m in w.messages() {
                labels.push(
                    author_name(m.author.as_ref().and_then(|u| state.members.get(u)), &l)
                        .to_string(),
                );
                labels.push(parser.parse(m).plain_text());
                for u in m.reply_users.iter() {
                    labels.push(user_name(state.members.get(u), &l));
                }
            }
        }
        seen_unknown |= labels.iter().any(|x| x == "Unknown");
        for x in &labels {
            assert!(
                !bad(x),
                "scenario {}: raw id or markup shown as {x:?}",
                s.name
            );
        }
    }
    assert!(
        seen_unknown,
        "the fixture must exercise the unresolved path"
    );
}

#[test]
fn closed_direct_messages_are_not_in_the_sidebar() {
    let s = scenario::by_name("unresolved_names").unwrap();
    let session = ScenarioSession::new(&s);
    let rows = build_rows(
        session.state(),
        None,
        &SidebarSettings::default(),
        &HashSet::new(),
        &en(),
    );
    let labels: Vec<String> = rows
        .iter()
        .map(|r| match r {
            SidebarRow::Header { label: h, .. } => format!("#{h}"),
            SidebarRow::Conversation { label, .. } | SidebarRow::ShowMore { label, .. } => {
                label.to_string()
            }
        })
        .collect();
    assert!(labels.contains(&"#Channels".to_string()), "{labels:?}");
    assert!(
        labels.contains(&"#Direct messages".to_string()),
        "{labels:?}"
    );
    // Eli Park (closed DM) and the closed group are absent; the open group
    // names people consistently.
    assert!(!labels.iter().any(|l| l.contains("Eli")), "{labels:?}");
    assert!(
        labels.contains(&"Ana Reyes, Dana Ortiz, scott.m".to_string()),
        "{labels:?}"
    );
}
