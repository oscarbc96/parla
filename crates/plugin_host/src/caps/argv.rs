//! `exec` argument shapes: a literal must equal its
//! argument; a `*` matches within that one argument, never `..`, `?`, `#`,
//! `://`, `%` or a leading `-`, so `repos/*` cannot become another path, a
//! query, a URL, a percent escape (`%2e%2e`) or a flag.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::manifest::{ArgPattern, ArgShape};

/// What a `*` never matches.
const FORBIDDEN: [&str; 5] = ["..", "?", "#", "://", "%"];

impl ArgShape {
    /// Whether `args` is this shape: same length, each argument matching
    /// its element.
    pub fn matches(&self, args: &[String]) -> bool {
        self.0.len() == args.len() && self.0.iter().zip(args).all(|(p, a)| p.matches(a))
    }
}

impl ArgPattern {
    /// Whether one argument matches this element.
    pub fn matches(&self, arg: &str) -> bool {
        match self {
            Self::Literal(l) => l == arg,
            // A flag only ever matches a literal. A forbidden token the
            // literal parts do not spell came from a `*` (alone or joined
            // with a neighbouring literal), so the argument is refused.
            Self::Glob(g) => {
                !arg.starts_with('-')
                    && !FORBIDDEN.iter().any(|t| arg.contains(t) && !g.contains(t))
                    && glob(g, arg, &FORBIDDEN)
            }
        }
    }
}

/// Whether `text` matches `pattern`, where `*` stands for any run of
/// characters that contains none of `forbidden`. Runs in
/// O(stars * len^2) on byte positions, no backtracking blow-up.
pub(super) fn glob(pattern: &str, text: &str, forbidden: &[&str]) -> bool {
    let t = text.as_bytes();
    let n = t.len();
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("").as_bytes();
    if !t.starts_with(first) {
        return false;
    }
    // limit[p]: the furthest end a `*` starting at p may reach.
    let mut limit = vec![n; n + 1];
    for i in (0..n).rev() {
        let bad = forbidden
            .iter()
            .filter(|f| t[i..].starts_with(f.as_bytes()))
            .map(|f| i + f.len() - 1)
            .min()
            .unwrap_or(n);
        limit[i] = bad.min(limit[i + 1]);
    }
    let rest: Vec<&[u8]> = parts.map(str::as_bytes).collect();
    if rest.is_empty() {
        return first.len() == n;
    }
    // at[p]: some way to match the pattern so far ends at byte p.
    let mut at = vec![false; n + 1];
    at[first.len()] = true;
    for (j, part) in rest.iter().enumerate() {
        let last = j + 1 == rest.len();
        let mut next = vec![false; n + 1];
        for p in (0..=n).filter(|&p| at[p]) {
            for q in p..=limit[p] {
                let end = q + part.len();
                if end <= n && t[q..].starts_with(part) && (!last || end == n) {
                    next[end] = true;
                }
            }
        }
        at = next;
    }
    at[n]
}

/// Stored as the manifest's list of strings.
impl Serialize for ArgShape {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.iter().map(ArgPattern::as_str))
    }
}

impl<'de> Deserialize<'de> for ArgShape {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let parts = Vec::<String>::deserialize(d)?;
        Ok(Self(parts.into_iter().map(ArgPattern::from_text).collect()))
    }
}
