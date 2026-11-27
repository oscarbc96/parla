//! Group mentions outside the timeline: previews and the text of an edit.

use std::sync::Arc;

use domain::test_support::msg;
use domain::{ComposedSpan, ComposedText, Inline, Mention, UserGroup, UserGroups, WorkspaceState};
use settings::WorkspaceKey;

fn groups() -> UserGroups {
    [UserGroup {
        id: "S1".into(),
        handle: "eng".into(),
        name: "Engineering".into(),
        member_count: 3,
        is_member: true,
    }]
    .into_iter()
    .map(|g| (g.id.clone(), g))
    .collect()
}

#[test]
fn previews_show_the_group_handle_not_its_id() {
    let mut m = msg("C1", "1.000001", "ping @S1 now");
    m.mentions = Arc::from([Mention::Group("S1".into())]);
    assert_eq!(
        components::preview_text(&domain::PlainParser, &m, &groups()),
        "ping @eng now"
    );
}

#[test]
fn an_edited_group_mention_shows_its_handle() {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 10);
    s.user_groups = groups();
    // What a provider decode gives for a label-less group token.
    let decoded = ComposedText::new(
        "hi @S1 and @S2",
        vec![
            ComposedSpan {
                range: 3..6,
                inline: Inline::Group {
                    id: "S1".into(),
                    label: None,
                },
            },
            ComposedSpan {
                range: 11..14,
                inline: Inline::Group {
                    id: "S2".into(),
                    label: Some("@old".into()),
                },
            },
        ],
    );
    let t = components::completion::relabel_decoded(&s, &decoded);
    assert_eq!(&*t.text, "hi @eng and @old");
    assert_eq!(t.spans[0].range, 3..7);
    assert_eq!(t.spans[1].range, 12..16);
}
