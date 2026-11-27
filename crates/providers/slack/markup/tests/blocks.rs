use domain::{
    Block, Broadcast, ContextItem, ControlKind, ImageRef, Inline, ListStyle, RemoteUrl, Style,
};
use serde_json::value::RawValue;
use slack_markup::{parse_attachments, parse_blocks, parse_message};

fn raw(s: &str) -> Box<RawValue> {
    RawValue::from_string(s.to_string()).unwrap()
}

fn fixture(name: &str) -> Box<RawValue> {
    let p = format!(
        "{}/tests/fixtures/blocks/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    raw(&std::fs::read_to_string(p).unwrap())
}

fn txt(s: &str) -> Inline {
    Inline::Text {
        text: s.into(),
        style: Style::default(),
    }
}

#[test]
fn section_with_fields_and_accessory() {
    let b = parse_blocks(&fixture("section_fields.json"));
    assert_eq!(b.len(), 3);
    let Block::Section {
        text,
        fields,
        accessory,
    } = &b[0]
    else {
        panic!("{:?}", b[0])
    };
    assert!(
        matches!(&text[0], Block::Paragraph(i) if matches!(&i[0], Inline::Text{style,..} if style.bold))
    );
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[1], vec![txt("ok")]);
    assert!(matches!(accessory.as_deref(), Some(Block::Image { alt, .. }) if &**alt == "logo"));
    assert_eq!(b[1], Block::Heading(vec![txt("Release")]));
    assert_eq!(b[2], Block::Divider);
}

#[test]
fn rich_text_styles_lists_and_quotes() {
    let b = parse_blocks(&fixture("rich_text.json"));
    let Block::Paragraph(i) = &b[0] else { panic!() };
    assert!(matches!(&i[0], Inline::Text{style,..} if style.bold));
    assert!(matches!(&i[1], Inline::Text{style,..} if style.italic && style.strike));
    assert_eq!(i[2], Inline::Code("x".into()));
    assert!(
        matches!(&i[3], Inline::Link{url,label,..} if &**url=="https://example.test" && label.as_deref()==Some("site"))
    );
    assert!(matches!(&i[4], Inline::User{id,..} if &*id.0=="U0000001"));
    assert!(matches!(&i[5], Inline::Conversation{id,..} if &*id.0=="C0000001"));
    assert!(matches!(&i[6], Inline::Group{id,..} if &**id=="S0000001"));
    assert_eq!(i[7], Inline::Broadcast(Broadcast::Here));
    assert!(matches!(&i[8], Inline::Date{unix:1700000000,fallback} if &**fallback=="Nov 14"));
    assert_eq!(
        b[1],
        Block::List {
            style: ListStyle::Ordered { start: 3 },
            indent: 1,
            items: vec![vec![txt("one")], vec![txt("two")]]
        }
    );
    assert!(matches!(
        &b[2],
        Block::List {
            style: ListStyle::Bullet,
            indent: 0,
            ..
        }
    ));
    assert_eq!(
        b[3],
        Block::Code {
            lang: None,
            text: "let a = 1;".into()
        }
    );
    assert_eq!(
        b[4],
        Block::Quote(vec![Block::Paragraph(vec![txt("quoted")])])
    );
}

#[test]
fn rich_text_emoji_uses_unicode_hex() {
    let j = r#"[{"type":"rich_text","elements":[{"type":"rich_text_section","elements":[
        {"type":"emoji","name":"smile","unicode":"1f642"},
        {"type":"emoji","name":"family","unicode":"1f468-200d-1f469"},
        {"type":"emoji","name":"partyparrot"}]}]}]"#;
    let b = parse_blocks(&raw(j));
    let Block::Paragraph(i) = &b[0] else { panic!() };
    let u = |n: usize| match &i[n] {
        Inline::Emoji(e) => e.unicode.as_deref().map(str::to_owned),
        o => panic!("{o:?}"),
    };
    assert_eq!(u(0).as_deref(), Some("\u{1f642}"));
    assert_eq!(u(1).as_deref(), Some("\u{1f468}\u{200d}\u{1f469}"));
    assert_eq!(u(2), None);
}

#[test]
fn actions_and_inputs_render_disabled_controls() {
    let j = r#"[
      {"type":"actions","elements":[
        {"type":"button","text":{"type":"plain_text","text":"Go"}},
        {"type":"static_select","placeholder":{"type":"plain_text","text":"Pick"}},
        {"type":"datepicker"}]},
      {"type":"input","label":{"type":"plain_text","text":"Name"},"element":{"type":"plain_text_input"}}]"#;
    let b = parse_blocks(&raw(j));
    let Block::Actions(c) = &b[0] else { panic!() };
    assert_eq!((&*c[0].label, c[0].kind), ("Go", ControlKind::Button));
    assert_eq!((&*c[1].label, c[1].kind), ("Pick", ControlKind::Select));
    assert_eq!(c[2].kind, ControlKind::Other);
    let Block::Actions(c) = &b[1] else { panic!() };
    assert_eq!((&*c[0].label, c[0].kind), ("Name", ControlKind::Other));
}

#[test]
fn image_block_web_and_provider_file() {
    let j = r#"[
      {"type":"image","image_url":"https://img.example.test/a.png","alt_text":"a","title":{"type":"plain_text","text":"T"}},
      {"type":"image","slack_file":{"url":"https://files.example.test/f"},"alt_text":"b"},
      {"type":"context","elements":[{"type":"image","image_url":"https://img.example.test/c.png","alt_text":"c"},{"type":"mrkdwn","text":"ctx"}]}]"#;
    let b = parse_blocks(&raw(j));
    assert!(
        matches!(&b[0], Block::Image{source:ImageRef::Web(u),title:Some(t),..} if &**u=="https://img.example.test/a.png" && &**t=="T")
    );
    assert!(
        matches!(&b[1], Block::Image{source:ImageRef::Provider(u),..} if *u==RemoteUrl("https://files.example.test/f".into()))
    );
    let Block::Context(items) = &b[2] else {
        panic!()
    };
    assert!(matches!(&items[0], ContextItem::Image { .. }));
    assert_eq!(items[1], ContextItem::Text(vec![txt("ctx")]));
}

#[test]
fn table_block() {
    let j = r#"[{"type":"table","header":true,"rows":[
      [{"type":"raw_text","text":"A"},{"type":"raw_text","text":"B"}],
      [{"type":"raw_text","text":"1"},{"type":"rich_text","elements":[{"type":"rich_text_section","elements":[{"type":"text","text":"2"}]}]}]]}]"#;
    let b = parse_blocks(&raw(j));
    assert_eq!(
        b[0],
        Block::Table {
            header: true,
            rows: vec![
                vec![vec![txt("A")], vec![txt("B")]],
                vec![vec![txt("1")], vec![txt("2")]]
            ]
        }
    );
}

#[test]
fn unknown_block_falls_back_to_text() {
    let b = parse_blocks(&fixture("mixed.json"));
    assert!(matches!(&b[2], Block::Unsupported{fallback} if fallback.is_empty()));
    assert!(matches!(&b[3], Block::Unsupported{fallback} if &**fallback=="Clip"));
    let b = parse_blocks(&raw(
        r#"[{"type":"hologram","text":{"type":"plain_text","text":"fb"}}]"#,
    ));
    assert!(matches!(&b[0], Block::Unsupported{fallback} if &**fallback=="fb"));
}

#[test]
fn malformed_block_keeps_neighbours() {
    let b = parse_blocks(&fixture("mixed.json"));
    assert_eq!(b.len(), 5);
    assert_eq!(b[0], Block::Divider);
    assert!(matches!(&b[1], Block::Unsupported { .. }));
    assert_eq!(b[4], Block::Heading(vec![txt("After")]));
    assert!(parse_blocks(&raw("{}")).is_empty());
    assert!(parse_blocks(&raw("null")).is_empty());
    let b = parse_blocks(&raw(r#"[{"type":"section","text":7,"fields":"x"}]"#));
    assert_eq!(b.len(), 1);
}

#[test]
fn attachment_unfurl_card() {
    let b = parse_attachments(&fixture("attachment_unfurl.json"));
    assert_eq!(b[0], Block::Paragraph(vec![txt("before")]));
    let Block::Card(c) = &b[1] else {
        panic!("{:?}", b[1])
    };
    assert_eq!(c.color, Some(0x36a64f));
    assert!(
        matches!(c.author.as_deref(), Some([Inline::Link{label,..}]) if label.as_deref()==Some("lorem"))
    );
    assert!(matches!(&c.title, Some(Inline::Link{url,..}) if &**url=="https://example.test/t"));
    assert!(
        matches!(&c.body[0], Block::Paragraph(i) if matches!(&i[0], Inline::Text{style,..} if style.bold))
    );
    assert_eq!(c.fields.len(), 1);
    assert_eq!(&*c.fields[0].0, "Open");
    assert!(matches!(&c.image, Some(ImageRef::Web(_))));
    assert!(matches!(&c.thumb, Some(ImageRef::Web(_))));
    assert!(
        matches!(c.footer.as_deref(), Some([Inline::Link{label,..}]) if label.as_deref()==Some("lorem"))
    );
}

#[test]
fn redacted_history_fixture_parses() {
    let p = format!(
        "{}/../api/tests/fixtures/spike/conversations.history.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let msgs = v["response"]["body"]["json"]["messages"]
        .as_array()
        .unwrap();
    let mut cards = 0;
    for m in msgs {
        let blocks = m.get("blocks").map(|b| raw(&b.to_string()));
        let atts = m
            .get("attachments")
            .filter(|a| !a.is_null())
            .map(|b| raw(&b.to_string()));
        let rt = parse_message(
            m["text"].as_str().unwrap_or(""),
            blocks.as_deref(),
            atts.as_deref(),
            None,
        );
        assert!(!rt.is_empty());
        cards += rt
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Card(_)))
            .count();
    }
    assert!(cards >= 2);
}

#[test]
fn rich_text_blocks_replace_fallback_text() {
    let blocks = fixture("rich_text.json");
    let rt = parse_message("FALLBACK TEXT", Some(&blocks), None, None);
    assert!(!rt.plain_text().contains("FALLBACK"));
    assert!(rt.plain_text().contains("hi there"));
}

#[test]
fn text_is_dropped_when_blocks_render() {
    let blocks = raw(
        r#"[{"type":"section","text":{"type":"mrkdwn","text":"Alert body"}},{"type":"divider"}]"#,
    );
    let atts = fixture("attachment_unfurl.json");
    let rt = parse_message(
        "Alert: 1 test needs attention",
        Some(&blocks),
        Some(&atts),
        None,
    );
    assert!(!rt.plain_text().contains("Alert: 1 test"));
    assert!(rt.plain_text().contains("Alert body"));
    assert!(matches!(rt.blocks.last(), Some(Block::Card(_))));
}

#[test]
fn text_renders_when_blocks_are_all_unsupported() {
    let blocks = raw(r#"[{"type":"video","title":{"type":"plain_text","text":"clip"}}]"#);
    let rt = parse_message("fallback *words*", Some(&blocks), None, None);
    assert!(rt.plain_text().contains("fallback"));
}

#[test]
fn text_renders_when_blocks_are_empty() {
    let blocks = raw("[]");
    let rt = parse_message("only text", Some(&blocks), None, None);
    assert!(rt.plain_text().contains("only text"));
}

fn has_emoji(i: &[Inline]) -> bool {
    i.iter().any(|x| matches!(x, Inline::Emoji(_)))
}

#[test]
fn attachment_text_fields_and_pretext_render_emoji_by_default() {
    let atts = raw(
        r#"[{"pretext":":tada: Done","text":":robot_face: Generated with a tool",
            "fields":[{"title":"Status","value":":white_check_mark: ok"}],"footer":":gear: bot"}]"#,
    );
    let b = parse_attachments(&atts);
    let Block::Paragraph(p) = &b[0] else {
        panic!("{:?}", b[0])
    };
    assert!(has_emoji(p));
    let Block::Card(c) = &b[1] else {
        panic!("{:?}", b[1])
    };
    let Block::Paragraph(body) = &c.body[0] else {
        panic!("{:?}", c.body)
    };
    assert!(has_emoji(body));
    assert!(has_emoji(&c.fields[0].1));
    assert!(has_emoji(c.footer.as_deref().unwrap()));
}

#[test]
fn attachment_mrkdwn_in_present_limits_parsing() {
    let atts = raw(r#"[{"text":":robot_face: raw","mrkdwn_in":["pretext"]}]"#);
    let b = parse_attachments(&atts);
    let Block::Card(c) = &b[0] else {
        panic!("{:?}", b[0])
    };
    let Block::Paragraph(body) = &c.body[0] else {
        panic!("{:?}", c.body)
    };
    assert!(!has_emoji(body));
}

fn text_of(i: &[Inline]) -> String {
    i.iter()
        .map(|x| match x {
            Inline::Text { text, .. } => text.to_string(),
            Inline::LineBreak => "\n".into(),
            _ => String::new(),
        })
        .collect()
}

#[test]
fn bot_attachment_collapses_blank_lines() {
    let atts = raw(
        r#"[{"pretext":"\n\nAlert fired\n\n\n","color":"danger","text":"\n\nCPU high\n\n\n\nDisk ok  \n \n\n\nLast\n\n",
            "fields":[{"title":"Host","value":"web-1\n\n\n\nweb-2\n"},{"title":"Env","value":"prod"}]}]"#,
    );
    let b = parse_attachments(&atts);
    let Block::Paragraph(p) = &b[0] else {
        panic!("{:?}", b[0])
    };
    assert_eq!(text_of(p), "Alert fired");
    let Block::Card(c) = &b[1] else {
        panic!("{:?}", b[1])
    };
    let body: Vec<String> = c
        .body
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => text_of(p),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(body, ["CPU high", "Disk ok", "Last"]);
    assert_eq!(text_of(&c.fields[0].1), "web-1\nweb-2");
}

#[test]
fn placeholder_only_attachment_is_hidden() {
    let atts = serde_json::value::RawValue::from_string(
        r#"[{"fallback":"[no preview available]","id":1},
            {"fallback":" [No preview available] ","color":"36a64f"},
            {"fallback":"[no preview available]","title":"Release notes","title_link":"https://example.com/r"},
            {"fallback":"Plain fallback","id":2}]"#
            .into(),
    )
    .unwrap();
    let b = parse_attachments(&atts);
    // The two placeholder-only attachments vanish; the titled one keeps its
    // title without the placeholder body; real fallbacks still show.
    assert_eq!(b.len(), 2, "{b:?}");
    let Block::Card(c) = &b[0] else {
        panic!("{b:?}")
    };
    assert!(c.title.is_some() && c.body.is_empty());
    let Block::Card(c) = &b[1] else {
        panic!("{b:?}")
    };
    assert!(!c.body.is_empty());
}

#[test]
fn ordered_list_offset_at_u32_max_does_not_overflow() {
    let j = r#"[{"type":"rich_text","elements":[{"type":"rich_text_list","style":"ordered","offset":4294967295,"elements":[
        {"type":"rich_text_section","elements":[{"type":"text","text":"x"}]}]}]}]"#;
    assert!(!parse_blocks(&raw(j)).is_empty());
}
