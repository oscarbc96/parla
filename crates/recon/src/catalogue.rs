//! Parser for the rows of a provider's `catalogue.md` (docs/providers/<id>/web-client).

/// One method of an http row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub method: String,
    /// The catalogue's Read-only column says exactly `yes`.
    pub read_only: bool,
    /// Fixture path chosen for this method, relative to the repository root.
    pub fixture: Option<String>,
}

fn strip_ticks(s: &str) -> &str {
    s.trim().trim_matches('`')
}

/// Parse every http row; ws rows, the header and separators are ignored.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cols: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cols.len() < 5 || cols[2] != "http" {
            continue;
        }
        let methods: Vec<&str> = cols[1].split(',').map(strip_ticks).collect();
        let fixtures: Vec<&str> = cols[4]
            .split(',')
            .map(strip_ticks)
            .filter(|f| f.ends_with(".json"))
            .collect();
        for m in &methods {
            if m.is_empty() {
                continue;
            }
            let stem_matches = |f: &&&str| {
                let f = f.replace("<method>", m);
                f.rsplit('/')
                    .next()
                    .is_some_and(|name| name.strip_suffix(".json") == Some(m))
            };
            let fixture = fixtures
                .iter()
                .find(stem_matches)
                .map(|f| f.replace("<method>", m))
                .or_else(|| {
                    (methods.len() == 1 && fixtures.len() == 1).then(|| fixtures[0].to_string())
                });
            out.push(Entry {
                method: (*m).to_string(),
                read_only: cols[3] == "yes",
                fixture,
            });
        }
    }
    out
}
