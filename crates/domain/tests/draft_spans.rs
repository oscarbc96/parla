use domain::{
    Broadcast, ComposedSpan, EmojiRef, Inline, Style, UserId, spans_from_json, spans_to_json,
};

fn sp(range: std::ops::Range<usize>, inline: Inline) -> ComposedSpan {
    ComposedSpan { range, inline }
}

fn all() -> (String, Vec<ComposedSpan>) {
    let text = "hi @Ana #dev :party: @here docs".to_string();
    let at = |s: &str| {
        let i = text.find(s).unwrap();
        i..i + s.len()
    };
    let spans = vec![
        sp(
            at("@Ana"),
            Inline::User {
                id: UserId("U1".into()),
                label: Some("Ana".into()),
            },
        ),
        sp(
            at("#dev"),
            Inline::Conversation {
                id: domain::ConversationId("C1".into()),
                label: None,
            },
        ),
        sp(
            at(":party:"),
            Inline::Emoji(EmojiRef {
                shortcode: "party".into(),
                unicode: None,
                skin_tone: Some(3),
            }),
        ),
        sp(at("@here"), Inline::Broadcast(Broadcast::Here)),
        sp(
            at("docs"),
            Inline::Link {
                url: "https://example.com/a?b=1&c=2".into(),
                label: Some("docs".into()),
                style: Style::default(),
            },
        ),
    ];
    (text, spans)
}

#[test]
fn spans_round_trip() {
    let (text, spans) = all();
    let json = spans_to_json(&spans).expect("json");
    assert_eq!(spans_from_json(&json, &text), spans);
}

#[test]
fn no_spans_is_none() {
    assert_eq!(spans_to_json(&[]), None);
}

#[test]
fn spans_outside_the_text_are_dropped() {
    let (text, spans) = all();
    let json = spans_to_json(&spans).unwrap();
    assert_eq!(spans_from_json(&json, "short"), vec![]);
    assert_eq!(spans_from_json("not json", &text), vec![]);
    let kept = spans_from_json(&json, &text[..text.find("@here").unwrap()]);
    assert_eq!(kept.len(), 3);
}

#[test]
fn relabel_swaps_span_text_and_shifts_later_spans() {
    use domain::{ComposedSpan, ComposedText, Inline, UserId};
    let user = |id: &str| Inline::User {
        id: UserId(id.into()),
        label: None,
    };
    let t = ComposedText::new(
        "a @U1 b @U2",
        vec![
            ComposedSpan {
                range: 2..5,
                inline: user("U1"),
            },
            ComposedSpan {
                range: 8..11,
                inline: user("U2"),
            },
        ],
    );
    let out = t.relabel(|i| match i {
        Inline::User { id, .. } if &*id.0 == "U1" => Some("@Ana Lopez".into()),
        _ => None,
    });
    assert_eq!(&*out.text, "a @Ana Lopez b @U2");
    assert_eq!(out.spans[0].range, 2..12);
    assert_eq!(out.spans[1].range, 15..18);
    assert_eq!(&out.text[out.spans[1].range.clone()], "@U2");
}
