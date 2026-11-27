//! Mention rule: `<@U..>`, `<#C..|name>` and `<!here>` keep their (mapped) shape.

use regex::Regex;

/// Whole-word mention. `keep` is redacted and kept, `tail` closes it; a `|label`
/// part is dropped.
pub fn mention() -> Regex {
    Regex::new(r"^(?P<keep><[@#!][^|>]*)(?:\|[^>]*)?(?P<tail>>)$").expect("static pattern compiles")
}
