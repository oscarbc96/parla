use domain::{
    Broadcast, ComposedSpan, ComposedText, ContentEncoder, ConversationId, EmojiRef, Inline, UserId,
};
use slack_markup::SlackEncoder;

fn span(range: std::ops::Range<usize>, inline: Inline) -> ComposedSpan {
    ComposedSpan { range, inline }
}

#[test]
fn encoder_escapes_user_text() {
    let t = ComposedText::plain("a<b & c>");
    assert_eq!(SlackEncoder.encode(&t), "a&lt;b &amp; c&gt;");
}

#[test]
fn typed_token_lookalike_stays_text() {
    let t = ComposedText::plain("<@U1> and <!here>");
    assert_eq!(SlackEncoder.encode(&t), "&lt;@U1&gt; and &lt;!here&gt;");
}

#[test]
fn encoder_writes_mention_tokens() {
    let text = "hi @Ana Lopez in #general :tada: @here @channel @everyone @design & <x>";
    let at = |s: &str| {
        let i = text.find(s).unwrap();
        i..i + s.len()
    };
    let spans = vec![
        span(
            at("@Ana Lopez"),
            Inline::User {
                id: UserId("U1".into()),
                label: None,
            },
        ),
        span(
            at("#general"),
            Inline::Conversation {
                id: ConversationId("C1".into()),
                label: None,
            },
        ),
        span(
            at(":tada:"),
            Inline::Emoji(EmojiRef {
                shortcode: "tada".into(),
                unicode: None,
                skin_tone: None,
            }),
        ),
        span(at("@here"), Inline::Broadcast(Broadcast::Here)),
        span(at("@channel"), Inline::Broadcast(Broadcast::Channel)),
        span(at("@everyone"), Inline::Broadcast(Broadcast::Everyone)),
        span(
            at("@design"),
            Inline::Group {
                id: "S9".into(),
                label: None,
            },
        ),
    ];
    let t = ComposedText::new(text, spans);
    assert_eq!(
        SlackEncoder.encode(&t),
        "hi <@U1> in <#C1> :tada: <!here> <!channel> <!everyone> <!subteam^S9> &amp; &lt;x&gt;"
    );
}

#[test]
fn span_label_is_not_escaped_twice_or_leaked() {
    let text = "@A&B x";
    let t = ComposedText::new(
        text,
        vec![span(
            0..4,
            Inline::User {
                id: UserId("U2".into()),
                label: None,
            },
        )],
    );
    assert_eq!(SlackEncoder.encode(&t), "<@U2> x");
}

#[test]
fn encoder_writes_links_in_slack_syntax() {
    let text = "see the docs & more";
    let i = text.find("the docs").unwrap();
    let link = |label: Option<&str>| {
        vec![span(
            i..i + "the docs".len(),
            Inline::Link {
                url: "https://example.com/a?b=1&c=<2>".into(),
                label: label.map(Into::into),
                style: Default::default(),
            },
        )]
    };
    let t = ComposedText::new(text, link(Some("the docs")));
    assert_eq!(
        SlackEncoder.encode(&t),
        "see <https://example.com/a?b=1&amp;c=&lt;2&gt;|the docs> &amp; more"
    );
    // A link whose text is its own address needs no label.
    let url = "https://example.com/x";
    let t = ComposedText::new(
        url,
        vec![span(
            0..url.len(),
            Inline::Link {
                url: url.into(),
                label: None,
                style: Default::default(),
            },
        )],
    );
    assert_eq!(SlackEncoder.encode(&t), "<https://example.com/x>");
}

#[test]
fn decoder_turns_tokens_back_into_spans() {
    let t = SlackEncoder.decode(
        "hi <@U1> in <#C1|general> <!here> <https://x.test/a?b=1&amp;c=2|docs> &lt;x&gt; *b*",
    );
    assert_eq!(&*t.text, "hi @U1 in #general @here docs <x> *b*");
    let kinds: Vec<_> = t
        .spans
        .iter()
        .map(|s| (&t.text[s.range.clone()], &s.inline))
        .collect();
    assert_eq!(kinds.len(), 4);
    assert_eq!(kinds[0].0, "@U1");
    assert!(matches!(kinds[0].1, Inline::User { id, .. } if id.0.as_ref() == "U1"));
    assert_eq!(kinds[1].0, "#general");
    assert!(matches!(kinds[1].1, Inline::Conversation { id, .. } if id.0.as_ref() == "C1"));
    assert_eq!(kinds[2].0, "@here");
    assert!(matches!(kinds[2].1, Inline::Broadcast(Broadcast::Here)));
    assert_eq!(kinds[3].0, "docs");
    assert!(
        matches!(kinds[3].1, Inline::Link { url, .. } if url.as_ref() == "https://x.test/a?b=1&c=2")
    );
}

#[test]
fn decode_then_encode_keeps_the_markup() {
    for m in [
        "plain *bold* _it_ ~s~ `c`",
        "ask <@U1> about <#C1> <!channel>",
        "see <https://x.test|the docs> and <https://y.test>",
        "a &lt;b&gt; &amp; c",
        "<!subteam^S1> go",
        "was <!date^1700000000^{date_short}|Nov 14> ok <!foo|bar>",
    ] {
        assert_eq!(SlackEncoder.encode(&SlackEncoder.decode(m)), m, "{m}");
    }
}

#[test]
fn group_mentions_decode_without_a_double_at() {
    let t = SlackEncoder.decode("hi <!subteam^S1|@eng> and <!subteam^S2>");
    assert_eq!(&*t.text, "hi @eng and @S2");
    assert_eq!(t.spans.len(), 2);
    assert_eq!(
        SlackEncoder.encode(&t),
        "hi <!subteam^S1> and <!subteam^S2>"
    );
}

#[test]
fn date_and_unknown_commands_survive_as_one_unit_each() {
    let m = "at <!date^1700000000^{date_short}|Nov 14> <!foo|bar>";
    let t = SlackEncoder.decode(m);
    assert_eq!(&*t.text, "at Nov 14 bar");
    assert_eq!(t.spans.len(), 2);
    assert_eq!(SlackEncoder.encode(&t), m);
}
