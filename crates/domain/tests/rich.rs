use domain::*;

fn t(s: &str) -> Inline {
    Inline::Text {
        text: s.into(),
        style: Style::default(),
    }
}

#[test]
fn plain_constructor_is_one_paragraph() {
    let r = RichText::plain("hi");
    assert_eq!(&*r.blocks, &[Block::Paragraph(vec![t("hi")])]);
    assert!(!r.is_empty());
}

#[test]
fn plain_text_flattens_blocks_and_inlines() {
    let r = RichText {
        blocks: vec![
            Block::Paragraph(vec![
                t("hi "),
                Inline::User {
                    id: UserId("U1".into()),
                    label: Some("ana".into()),
                },
                t(" "),
                Inline::Emoji(EmojiRef {
                    shortcode: "+1".into(),
                    unicode: Some("\u{1F44D}".into()),
                    skin_tone: None,
                }),
            ]),
            Block::Code {
                lang: None,
                text: "x".into(),
            },
        ]
        .into(),
    };
    assert_eq!(r.plain_text(), "hi @ana \u{1F44D}\nx");
}

#[test]
fn plain_text_custom_emoji_keeps_shortcode() {
    let r = RichText {
        blocks: vec![Block::Paragraph(vec![Inline::Emoji(EmojiRef {
            shortcode: "party-blob".into(),
            unicode: None,
            skin_tone: None,
        })])]
        .into(),
    };
    assert_eq!(r.plain_text(), ":party-blob:");
}

#[test]
fn plain_text_skips_controls() {
    let r = RichText {
        blocks: vec![
            Block::Actions(vec![Control {
                label: "Go".into(),
                kind: ControlKind::Button,
            }]),
            Block::Paragraph(vec![t("a")]),
        ]
        .into(),
    };
    assert_eq!(r.plain_text(), "a");
}

#[test]
fn remote_url_debug_hides_path() {
    let u = RemoteUrl("https://files.example.test/a/secret.png".into());
    let d = format!("{u:?}");
    assert!(d.contains("files.example.test") && !d.contains("secret"));
}
