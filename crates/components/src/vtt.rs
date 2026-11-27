//! WebVTT captions for clips: a small parser for the cue timings and text,
//! and a lookup of the cue showing at a position. Styling, regions and cue
//! settings are ignored; tags are stripped and the common entities decoded.

use gpui::SharedString;

/// Largest caption file read (bytes). A clip's captions are a few KB; a
/// bigger file is not captions and is not parsed.
pub const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    /// Exclusive.
    pub end_ms: u64,
    pub text: SharedString,
}

/// Cues of a WebVTT file, sorted by start. A file without the `WEBVTT`
/// header yields none; a cue with a bad or backwards timing is skipped.
pub fn parse(src: &str) -> Vec<Cue> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let src = src.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = src.split("\n\n").map(|b| b.trim_matches('\n'));
    let header = blocks.next().unwrap_or("");
    let first = header.lines().next().unwrap_or("");
    if first != "WEBVTT" && !first.starts_with("WEBVTT ") && !first.starts_with("WEBVTT\t") {
        return Vec::new();
    }
    let mut cues: Vec<Cue> = blocks.filter_map(cue).collect();
    cues.sort_by_key(|c| c.start_ms);
    cues
}

fn cue(block: &str) -> Option<Cue> {
    let mut lines = block.lines();
    let mut timing = lines.next()?;
    if !timing.contains("-->") {
        // An identifier line comes first; NOTE, STYLE and REGION blocks have
        // no timing at all and are skipped here.
        timing = lines.next()?;
    }
    let (start, rest) = timing.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    let (start_ms, end_ms) = (timestamp(start.trim())?, timestamp(end)?);
    if end_ms <= start_ms {
        return None;
    }
    let text = lines.map(clean).collect::<Vec<_>>().join("\n");
    Some(Cue {
        start_ms,
        end_ms,
        text: SharedString::from(text),
    })
}

/// `hh:mm:ss.ttt` or `mm:ss.ttt` in milliseconds.
fn timestamp(s: &str) -> Option<u64> {
    let (clock, frac) = s.split_once('.')?;
    if frac.len() != 3 {
        return None;
    }
    let ms: u64 = frac.parse().ok()?;
    let mut parts = clock.split(':');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next());
    if parts.next().is_some() {
        return None;
    }
    let (h, m, sec) = match c {
        // Hours are not bounded by the format; checked arithmetic below
        // rejects any that overflow.
        Some(s) => (a.parse::<u64>().ok()?, b, s),
        None => (0, a, b),
    };
    if m.len() != 2 || sec.len() != 2 {
        return None;
    }
    let (m, sec): (u64, u64) = (m.parse().ok()?, sec.parse().ok()?);
    if m > 59 || sec > 59 {
        return None;
    }
    h.checked_mul(3_600_000)?
        .checked_add((m * 60 + sec) * 1000 + ms)
}

/// Strips `<...>` tags and decodes the entities WebVTT defines.
fn clean(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_tag = false;
    for ch in line.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", "\u{a0}")
        .replace("&lrm;", "\u{200e}")
        .replace("&rlm;", "\u{200f}")
        .replace("&amp;", "&")
}

/// Cues sorted by start, with the latest end reached so far at each index,
/// so a lookup finds a long cue behind any number of short ones.
#[derive(Clone, Debug, Default)]
pub struct Captions {
    cues: Vec<Cue>,
    /// `reach[i]` is the largest `end_ms` of `cues[..=i]`.
    reach: Vec<u64>,
}

impl Captions {
    /// `cues` sorted by start, as [`parse`] returns them.
    pub fn new(cues: Vec<Cue>) -> Self {
        let mut max = 0;
        let reach = cues
            .iter()
            .map(|c| {
                max = max.max(c.end_ms);
                max
            })
            .collect();
        Self { cues, reach }
    }

    /// Index of the cue showing at `ms` (`start <= ms < end`), the
    /// latest-starting one when cues overlap.
    pub fn index_at(&self, ms: u64) -> Option<usize> {
        let n = self.cues.partition_point(|c| c.start_ms <= ms);
        // Walk back only while some earlier cue still reaches past `ms`.
        (0..n)
            .rev()
            .take_while(|&i| self.reach[i] > ms)
            .find(|&i| ms < self.cues[i].end_ms)
    }

    /// The cue showing at `ms`.
    pub fn at(&self, ms: u64) -> Option<&Cue> {
        self.index_at(ms).map(|i| &self.cues[i])
    }
}
