use components::vtt::{Captions, Cue, parse};

const SAMPLE: &str = "\u{feff}WEBVTT - clip captions

NOTE written by hand
for the test

STYLE
::cue { color: red }

1
00:00:00.500 --> 00:00:02.000 align:start
<v Ada>Hello &amp; welcome</v>

00:02.000 --> 00:04.250
Two lines,
the second &lt;here&gt;

intro-3
01:00:00.000 --> 01:00:01.000
An hour in.
";

#[test]
fn parses_cues_skipping_notes_and_styles() {
    let cues = parse(SAMPLE);
    assert_eq!(
        cues,
        vec![
            Cue {
                start_ms: 500,
                end_ms: 2_000,
                text: "Hello & welcome".into(),
            },
            Cue {
                start_ms: 2_000,
                end_ms: 4_250,
                text: "Two lines,\nthe second <here>".into(),
            },
            Cue {
                start_ms: 3_600_000,
                end_ms: 3_601_000,
                text: "An hour in.".into(),
            },
        ]
    );
}

#[test]
fn rejects_files_without_the_header_and_bad_timings() {
    assert!(parse("1\n00:00.000 --> 00:01.000\nNo header\n").is_empty());
    let bad = "WEBVTT\n\n00:00.000 --> soon\nBroken\n\n00:00:01.000 --> 00:00:00.500\nBackwards\n\n00:00:01.000 --> 00:00:02.000\nGood\n";
    let cues = parse(bad);
    assert_eq!(cues.len(), 1);
    assert_eq!(&*cues[0].text, "Good");
}

#[test]
fn windows_line_endings_parse() {
    let cues = parse("WEBVTT\r\n\r\n00:00.000 --> 00:01.000\r\nHi\r\n");
    assert_eq!(cues.len(), 1);
    assert_eq!(&*cues[0].text, "Hi");
}

#[test]
fn cue_lookup_by_position() {
    let cues = Captions::new(parse(SAMPLE));
    fn cue_at(c: &Captions, ms: u64) -> Option<&Cue> {
        c.at(ms)
    }
    assert_eq!(cue_at(&cues, 0), None);
    assert_eq!(cue_at(&cues, 499), None);
    assert_eq!(
        cue_at(&cues, 500).map(|c| &*c.text),
        Some("Hello & welcome")
    );
    assert_eq!(cue_at(&cues, 1_999).map(|c| c.start_ms), Some(500));
    // The end is exclusive; the next cue starts there.
    assert_eq!(cue_at(&cues, 2_000).map(|c| c.start_ms), Some(2_000));
    assert_eq!(cue_at(&cues, 4_250), None);
    assert_eq!(
        cue_at(&cues, 3_600_500).map(|c| c.start_ms),
        Some(3_600_000)
    );
    assert_eq!(Captions::new(Vec::new()).at(10), None);
}

#[test]
fn cues_come_out_sorted_by_start() {
    let cues = parse("WEBVTT\n\n00:05.000 --> 00:06.000\nB\n\n00:01.000 --> 00:02.000\nA\n");
    assert_eq!(cues[0].start_ms, 1_000);
    assert_eq!(Captions::new(cues).at(5_500).map(|c| &*c.text), Some("B"));
}

#[test]
fn huge_hours_are_rejected_not_overflowed() {
    let cues = parse(
        "WEBVTT\n\n99999999999999:00:00.000 --> 99999999999999:00:01.000\nToo far\n\n00:01.000 --> 00:02.000\nFine\n",
    );
    assert_eq!(cues.len(), 1);
    assert_eq!(&*cues[0].text, "Fine");
}

#[test]
fn a_long_cue_shows_behind_many_short_ones() {
    let mut src = String::from("WEBVTT\n\n00:00.000 --> 00:50.000\nSpeaker one\n");
    for i in 0..40 {
        src.push_str(&format!(
            "\n00:{:02}.000 --> 00:{:02}.500\nshort {i}\n",
            i + 1,
            i + 1
        ));
    }
    let c = Captions::new(parse(&src));
    // Inside a short cue: the latest-starting one.
    assert_eq!(c.at(30_200).map(|c| &*c.text), Some("short 29"));
    // Between the short cues, deep into the list: the long one still shows.
    assert_eq!(c.at(30_700).map(|c| &*c.text), Some("Speaker one"));
    assert_eq!(c.at(49_999).map(|c| &*c.text), Some("Speaker one"));
    assert_eq!(c.at(50_000), None);
}
