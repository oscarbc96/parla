//! Block structure: code fences, quotes, lists and paragraphs.

use domain::{Block, Inline, ListStyle};

use super::inline::inline;

#[derive(PartialEq)]
enum Item {
    Bullet,
    Ordered(u32),
}

struct Builder {
    out: Vec<Block>,
    para: Vec<Vec<Inline>>,
    quote: Vec<Vec<Inline>>,
    list: Option<(ListStyle, u8, Vec<Vec<Inline>>)>,
}

impl Builder {
    fn flush(&mut self) {
        if !self.para.is_empty() {
            self.out
                .push(Block::Paragraph(join(std::mem::take(&mut self.para))));
        }
        if !self.quote.is_empty() {
            let p = Block::Paragraph(join(std::mem::take(&mut self.quote)));
            self.out.push(Block::Quote(vec![p]));
        }
        if let Some((style, indent, items)) = self.list.take() {
            self.out.push(Block::List {
                style,
                indent,
                items,
            });
        }
    }
}

fn join(lines: Vec<Vec<Inline>>) -> Vec<Inline> {
    let mut out = Vec::new();
    for (n, l) in lines.into_iter().enumerate() {
        if n > 0 {
            out.push(Inline::LineBreak);
        }
        out.extend(l);
    }
    out
}

fn quote_prefix(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("&gt;")
        .or_else(|| line.strip_prefix('>'))?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn list_marker(line: &str) -> Option<(Item, u8, &str)> {
    let body = line.trim_start_matches([' ', '\t']);
    let ws = &line[..line.len() - body.len()];
    let width: usize = ws.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
    let indent = u8::try_from(width / 4).unwrap_or(u8::MAX);
    for m in ["• ", "- "] {
        if let Some(r) = body.strip_prefix(m) {
            return Some((Item::Bullet, indent, r));
        }
    }
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits)
        && let Some(r) = body[digits..].strip_prefix(". ")
    {
        return Some((Item::Ordered(body[..digits].parse().ok()?), indent, r));
    }
    None
}

pub(super) fn blocks_of(seg: &str, out: &mut Vec<Block>) {
    let mut b = Builder {
        out: Vec::new(),
        para: Vec::new(),
        quote: Vec::new(),
        list: None,
    };
    let mut quote_all = false;
    for line in seg.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if quote_all {
            b.quote.push(inline(line));
            continue;
        }
        if line.trim().is_empty() {
            b.flush();
            continue;
        }
        if let Some(rest) = quote_prefix(line) {
            b.flush_non_quote();
            if let Some(all) = rest
                .strip_prefix("&gt;&gt;")
                .or_else(|| rest.strip_prefix(">>"))
                && (line.starts_with(">>>") || line.starts_with("&gt;&gt;&gt;"))
            {
                quote_all = true;
                b.quote.push(inline(all.strip_prefix(' ').unwrap_or(all)));
                continue;
            }
            b.quote.push(inline(rest));
            continue;
        }
        if let Some((item, indent, rest)) = list_marker(line) {
            b.flush_non_list();
            let style = match item {
                Item::Bullet => ListStyle::Bullet,
                Item::Ordered(n) => ListStyle::Ordered { start: n },
            };
            let same = matches!(&b.list, Some((s, i, _))
                if *i == indent && std::mem::discriminant(s) == std::mem::discriminant(&style));
            if !same {
                if let Some((s, i, items)) = b.list.take() {
                    b.out.push(Block::List {
                        style: s,
                        indent: i,
                        items,
                    });
                }
                b.list = Some((style, indent, Vec::new()));
            }
            if let Some((_, _, items)) = &mut b.list {
                items.push(inline(rest));
            }
            continue;
        }
        b.flush_non_para();
        b.para.push(inline(line));
    }
    b.flush();
    out.extend(b.out);
}

impl Builder {
    fn flush_non_quote(&mut self) {
        let q = std::mem::take(&mut self.quote);
        self.flush();
        self.quote = q;
    }
    fn flush_non_list(&mut self) {
        let l = self.list.take();
        self.flush();
        self.list = l;
    }
    fn flush_non_para(&mut self) {
        let p = std::mem::take(&mut self.para);
        self.flush();
        self.para = p;
    }
}
