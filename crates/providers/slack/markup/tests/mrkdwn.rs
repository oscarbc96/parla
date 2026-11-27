use std::time::{Duration, Instant};

use domain::{
    Block, Broadcast, ConversationId, EmojiRef, Inline, ListStyle, Mention, Style, UserId,
};
use proptest::prelude::*;
use slack_markup::{emoji_ref, mentions, parse_mrkdwn, plain_text};

fn blocks(s: &str) -> Vec<Block> {
    parse_mrkdwn(s).blocks.to_vec()
}

fn inlines(s: &str) -> Vec<Inline> {
    match blocks(s).into_iter().next() {
        Some(Block::Paragraph(i)) => i,
        other => panic!("expected paragraph, got {other:?}"),
    }
}

fn t(s: &str) -> Inline {
    Inline::Text {
        text: s.into(),
        style: Style::default(),
    }
}
fn st(s: &str, bold: bool, italic: bool, strike: bool) -> Inline {
    Inline::Text {
        text: s.into(),
        style: Style {
            bold,
            italic,
            strike,
        },
    }
}

#[test]
fn bold_italic_strike() {
    assert_eq!(
        inlines("a *b* _c_ ~d~ e"),
        vec![
            t("a "),
            st("b", true, false, false),
            t(" "),
            st("c", false, true, false),
            t(" "),
            st("d", false, false, true),
            t(" e")
        ]
    );
    assert_eq!(inlines("*_x_*"), vec![st("x", true, true, false)]);
}

#[test]
fn bold_requires_word_boundary() {
    assert_eq!(inlines("a*b*c"), vec![t("a*b*c")]);
    assert_eq!(inlines("2*3*4"), vec![t("2*3*4")]);
    assert_eq!(inlines("snake_case_name"), vec![t("snake_case_name")]);
    assert_eq!(inlines("* a *"), vec![t("* a *")]);
}

#[test]
fn formatting_does_not_cross_lines() {
    assert_eq!(inlines("*a\nb*"), vec![t("*a"), Inline::LineBreak, t("b*")]);
}

#[test]
fn code_span_and_block_disable_formatting() {
    assert_eq!(
        inlines("x `*a* <@U1>` y"),
        vec![t("x "), Inline::Code("*a* <@U1>".into()), t(" y")]
    );
    let b = blocks("before\n```\n*a* &lt;b&gt;\nline2\n```\nafter");
    assert_eq!(b.len(), 3);
    assert_eq!(
        b[1],
        Block::Code {
            lang: None,
            text: "*a* <b>\nline2".into()
        }
    );
    // unterminated fence stays text
    assert_eq!(inlines("```oops"), vec![t("```oops")]);
}

#[test]
fn user_channel_group_broadcast_tokens() {
    assert_eq!(
        inlines("<@U1> <@U2|ana> <#C1|general> <!here> <!channel> <!everyone> <!subteam^S1|@devs>"),
        vec![
            Inline::User {
                id: UserId("U1".into()),
                label: None
            },
            t(" "),
            Inline::User {
                id: UserId("U2".into()),
                label: Some("ana".into())
            },
            t(" "),
            Inline::Conversation {
                id: ConversationId("C1".into()),
                label: Some("general".into())
            },
            t(" "),
            Inline::Broadcast(Broadcast::Here),
            t(" "),
            Inline::Broadcast(Broadcast::Channel),
            t(" "),
            Inline::Broadcast(Broadcast::Everyone),
            t(" "),
            Inline::Group {
                id: "S1".into(),
                label: Some("@devs".into())
            },
        ]
    );
}

#[test]
fn date_token_keeps_fallback() {
    assert_eq!(
        inlines("<!date^1392734382^{date_short} at {time}|Feb 18, 2014>"),
        vec![Inline::Date {
            unix: 1392734382,
            fallback: "Feb 18, 2014".into()
        }]
    );
}

#[test]
fn links_with_and_without_labels() {
    let plain = Style::default();
    assert_eq!(
        inlines("<https://a.test/x?a=1&amp;b=2> <https://b.test|B> <mailto:a@b.test|a@b.test>"),
        vec![
            Inline::Link {
                url: "https://a.test/x?a=1&b=2".into(),
                label: None,
                style: plain
            },
            t(" "),
            Inline::Link {
                url: "https://b.test".into(),
                label: Some("B".into()),
                style: plain
            },
            t(" "),
            Inline::Link {
                url: "mailto:a@b.test".into(),
                label: Some("a@b.test".into()),
                style: plain
            },
        ]
    );
    assert_eq!(inlines("1 < 2"), vec![t("1 < 2")]);
}

#[test]
fn entities_decoded_once() {
    assert_eq!(inlines("&amp;lt;"), vec![t("&lt;")]);
    assert_eq!(inlines("a &lt;b&gt; &amp; c"), vec![t("a <b> & c")]);
}

#[test]
fn quotes_and_lists() {
    let q = |s: &str| Block::Quote(vec![Block::Paragraph(vec![t(s)])]);
    assert_eq!(blocks("&gt; hi"), vec![q("hi")]);
    assert_eq!(blocks("> hi"), vec![q("hi")]);
    assert_eq!(
        blocks("&gt; a\n&gt; b"),
        vec![Block::Quote(vec![Block::Paragraph(vec![
            t("a"),
            Inline::LineBreak,
            t("b")
        ])])]
    );
    // nested quote markers are flattened to text
    assert_eq!(blocks("> > x"), vec![q("> x")]);
    assert_eq!(
        blocks("• one\n• two\n\n1. a\n2. b"),
        vec![
            Block::List {
                style: ListStyle::Bullet,
                indent: 0,
                items: vec![vec![t("one")], vec![t("two")]]
            },
            Block::List {
                style: ListStyle::Ordered { start: 1 },
                indent: 0,
                items: vec![vec![t("a")], vec![t("b")]]
            },
        ]
    );
    assert_eq!(
        blocks("- x"),
        vec![Block::List {
            style: ListStyle::Bullet,
            indent: 0,
            items: vec![vec![t("x")]]
        }]
    );
}

#[test]
fn emoji_shortcodes_and_skin_tones() {
    let e = |i: &Inline| match i {
        Inline::Emoji(e) => e.clone(),
        o => panic!("not emoji {o:?}"),
    };
    let up = e(&inlines(":thumbsup:")[0]);
    assert_eq!(up.unicode.as_deref(), Some("\u{1F44D}"));
    let toned = e(&inlines(":+1::skin-tone-3:")[0]);
    assert_eq!(toned.skin_tone, Some(3));
    assert_ne!(toned.unicode.as_deref(), Some("\u{1F44D}"));
    assert!(toned.unicode.is_some());
    let custom = e(&inlines(":party-blob:")[0]);
    assert_eq!(
        custom,
        EmojiRef {
            shortcode: "party-blob".into(),
            unicode: None,
            skin_tone: None
        }
    );
    assert_eq!(emoji_ref("+1::skin-tone-3"), toned);
    assert_eq!(emoji_ref("thumbsup").unicode.as_deref(), Some("\u{1F44D}"));
    // times are not emoji
    assert_eq!(inlines("at 12:30:45 ok"), vec![t("at 12:30:45 ok")]);
}

#[test]
fn emoji_glued_to_text() {
    let emoji = |s: &str| {
        inlines(s)
            .iter()
            .filter_map(|i| match i {
                Inline::Emoji(e) => Some(e.shortcode.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(emoji("Hey everyone:wave:"), vec!["wave".to_string()]);
    assert_eq!(
        emoji("email us at x@y.io.:custom-name:"),
        vec!["custom-name".to_string()]
    );
    assert_eq!(
        emoji("ok:tada::fire: go"),
        vec!["tada".to_string(), "fire".to_string()]
    );
    let toned = inlines("Hi:wave::skin-tone-3:");
    assert!(matches!(&toned[1], Inline::Emoji(e) if e.skin_tone == Some(3)));
    assert_eq!(toned.len(), 2);
    // not inside URLs, times, or a:b:c
    assert!(emoji("see https://a.io/x:wave:y and http://h/:wave:").is_empty());
    assert!(emoji("at 10:30:00 sharp").is_empty());
    assert!(emoji("key:value:more").is_empty());
    assert_eq!(inlines("key:value:more"), vec![t("key:value:more")]);
}

#[test]
fn mentions_extracted() {
    assert_eq!(
        mentions("<@U1> hi <@U2|b> <@U1> <!here> <!subteam^S9|@x> `<@U3>`"),
        vec![
            Mention::User(UserId("U1".into())),
            Mention::User(UserId("U2".into())),
            Mention::Broadcast(Broadcast::Here),
            Mention::Group("S9".into()),
        ]
    );
}

#[test]
fn plain_text_uses_labels() {
    assert_eq!(
        plain_text(
            "hi <@U1|ana> in <#C1|dev> <!here> :thumbsup: :party-blob: &amp; <https://x.test|site>"
        ),
        "hi @ana in #dev @here \u{1F44D} :party-blob: & site"
    );
    assert_eq!(plain_text("<@U1>"), "@U1");
}

fn adversarial() -> Vec<String> {
    let corpus = include_str!("fixtures/mrkdwn_corpus.txt");
    let mut v: Vec<String> = corpus.lines().map(str::to_owned).collect();
    v.push("*_~`<>".repeat(100_000 / 6));
    v.push("*".repeat(100_000));
    v.push("`".repeat(100_000));
    v.push("<".repeat(100_000));
    v.push(":a".repeat(50_000));
    v.push("*a ".repeat(30_000));
    v.push("> ".repeat(10_000));
    v.push(">".repeat(10_000));
    v.push("&gt;".repeat(10_000));
    v.push("\n>".repeat(10_000));
    v.push("```".repeat(30_000));
    v.push("- ".repeat(20_000));
    v.push(format!("{}x{}", "*_~".repeat(10_000), "~_*".repeat(10_000)));
    v
}

#[test]
fn parser_never_panics_on_corpus() {
    for s in adversarial() {
        let r = parse_mrkdwn(&s);
        let _ = r.plain_text();
        let _ = mentions(&s);
    }
}

/// Linear-time guard that does not depend on machine load: parsing 4x the
/// input must not take more than 10x the time (quadratic would be 16x).
/// Best of 7 runs for each size, with a 25 ms floor for scheduler noise.
#[test]
#[ignore = "timing-sensitive; run with `mise run test:perf`"]
fn parser_scales_linearly() {
    fn best(s: &str) -> Duration {
        (0..7)
            .map(|_| {
                let start = Instant::now();
                let r = parse_mrkdwn(s);
                let _ = r.plain_text();
                let _ = mentions(s);
                start.elapsed()
            })
            .min()
            .unwrap()
    }
    for unit in [
        "*_~",
        "- ",
        "<@U1|",
        "```",
        "&gt;",
        ":a",
        "ab:wave: ",
        "http://h:x:",
        "a:b:c ",
    ] {
        let small = unit.repeat(5_000);
        let large = unit.repeat(20_000);
        let (a, b) = (best(&small), best(&large));
        assert!(
            b <= a * 10 + Duration::from_millis(25),
            "unit {unit:?}: {a:?} for 5k, {b:?} for 20k"
        );
    }
}

/// CPU time of this thread, so a loaded machine (a shared CI runner, a busy
/// developer laptop) does not inflate the number: wall clock counts the time
/// the thread spent waiting for a core, CPU time only counts the parse.
fn cpu_now() -> Duration {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        #[repr(C)]
        struct Timespec {
            tv_sec: i64,
            tv_nsec: i64,
        }
        unsafe extern "C" {
            fn clock_gettime(clock_id: i32, tp: *mut Timespec) -> i32;
        }
        // `CLOCK_THREAD_CPUTIME_ID` is 16 on Darwin and 3 on Linux.
        const CLOCK: i32 = if cfg!(target_os = "macos") { 16 } else { 3 };
        let mut ts = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `clock_gettime` writes one `timespec` into a live, correctly
        // sized buffer.
        let r = unsafe { clock_gettime(CLOCK, &mut ts) };
        assert_eq!(r, 0, "clock_gettime(CLOCK_THREAD_CPUTIME_ID)");
        Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // No thread CPU clock: wall clock since the first reading, which is
        // monotonic and comparable between two calls.
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }
}

/// Throughput budget for emoji next to words and colon-heavy text. Measured
/// in CPU time on a debug build: about 76 ms for this input, 278 ms with the
/// per-colon look-behind scan (the regression this guards). Best of 5,
/// 150 ms ceiling.
#[test]
#[ignore = "timing-sensitive; run with `mise run test:perf`"]
fn emoji_text_stays_fast() {
    let lines = [
        "Hey everyone:wave: see https://a.io/x:y:z at 10:30:00 ok:tada::fire: go\n",
        "key:value:more :a:b:c: ab:wave::skin-tone-3: x\n",
    ]
    .concat()
    .repeat(1_500);
    // Long colon runs with no whitespace are what a look-behind scan hates.
    let input = format!("{lines}{}\n{lines}", ":a".repeat(30_000));
    let best = (0..5)
        .map(|_| {
            let start = cpu_now();
            let r = parse_mrkdwn(&input);
            let _ = r.plain_text();
            cpu_now() - start
        })
        .min()
        .unwrap();
    assert!(
        best < Duration::from_millis(150),
        "{} bytes took {best:?} of CPU",
        input.len()
    );
}

fn collect_texts(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                match (k.as_str(), v) {
                    ("text", serde_json::Value::String(s)) => out.push(s.clone()),
                    _ => collect_texts(v, out),
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|v| collect_texts(v, out)),
        _ => {}
    }
}

#[test]
fn real_fixture_messages_parse() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../api/tests/fixtures/spike");
    let mut texts = Vec::new();
    for f in [
        "conversations.history.json",
        "conversations.replies.json",
        "drafts.create.json",
    ] {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{dir}/{f}")).unwrap()).unwrap();
        collect_texts(&v, &mut texts);
    }
    assert!(texts.len() > 50);
    let mut seen_mention = false;
    for text in &texts {
        let r = parse_mrkdwn(text);
        if text.contains("<@U") {
            seen_mention = true;
            assert!(!mentions(text).is_empty(), "{text}");
            assert!(r.plain_text().contains('@'));
            assert!(!r.plain_text().contains("<@"));
        }
    }
    assert!(seen_mention);
}

#[test]
fn snapshots() {
    let msg = "*Deploy* done by <@U1|ana> in <#C1|ops> :tada:\n&gt; quoted &amp; _kept_\n• one\n• two\n```\nlet x = 1 &lt; 2;\n```\nsee <https://x.test/a|docs> <!date^1392734382^{date}|Feb 18> <!here>";
    insta::assert_debug_snapshot!(parse_mrkdwn(msg));
}

proptest! {
    #[test]
    fn never_panics_special_chars(s in "[*_~`<>:&;|^!@#a-z0-9 \n>•.-]{0,400}") {
        let r = parse_mrkdwn(&s);
        let _ = r.plain_text();
        let _ = mentions(&s);
    }

    #[test]
    fn never_panics_any_unicode(s in "\\PC{0,400}") {
        let _ = plain_text(&s);
        let _ = emoji_ref(&s);
    }
}

#[test]
fn triple_quote_quotes_the_rest() {
    let b = blocks("&gt;&gt;&gt; a\nb\n\nc");
    assert_eq!(
        b,
        vec![Block::Quote(vec![Block::Paragraph(vec![
            t("a"),
            Inline::LineBreak,
            t("b"),
            Inline::LineBreak,
            Inline::LineBreak,
            t("c")
        ])])]
    );
}

#[test]
fn text_only_message_formats_every_token() {
    // A message with no blocks: only `text` is parsed.
    let rt = slack_markup::parse_message(
        "*bold* _italic_ ~old~ `code` <@U0119> <#C0001|general> <https://example.com/p/1|link> :tada:",
        None,
        None,
        None,
    );
    let Block::Paragraph(i) = &rt.blocks[0] else {
        panic!("paragraph")
    };
    assert!(i.contains(&st("bold", true, false, false)));
    assert!(i.contains(&st("italic", false, true, false)));
    assert!(i.contains(&st("old", false, false, true)));
    assert!(i.contains(&Inline::Code("code".into())));
    assert!(
        i.iter()
            .any(|x| matches!(x, Inline::User { id, .. } if &*id.0 == "U0119"))
    );
    assert!(
        i.iter()
            .any(|x| matches!(x, Inline::Conversation { id, .. } if &*id.0 == "C0001"))
    );
    assert!(
        i.iter()
            .any(|x| matches!(x, Inline::Link { label: Some(l), .. } if &**l == "link"))
    );
    assert!(
        i.iter()
            .any(|x| matches!(x, Inline::Emoji(e) if &*e.shortcode == "tada"))
    );
}

#[test]
fn the_shrug_renders_literally_with_both_underscores() {
    let shrug = "no idea \u{af}\\_(\u{30c4})_/\u{af}";
    assert_eq!(inlines(shrug), vec![t(shrug)]);
    // A real italic next to it still works.
    assert!(
        inlines(&format!("_it_ {shrug}"))
            .iter()
            .any(|i| matches!(i, Inline::Text { style, .. } if style.italic))
    );
}
