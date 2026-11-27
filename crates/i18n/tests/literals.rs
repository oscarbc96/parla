//! Prose lint: user-facing text must live in catalogs, not in Rust literals.
//! Parses every `.rs` file of the listed crates with `syn` and flags string
//! literals that look like prose. Exemptions live in `i18n/allow-literals.toml`.

use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

const SKIP_MACROS: &[&str] = &[
    "log",
    "tracing",
    "trace",
    "debug",
    "info",
    "warn",
    "error",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "actions",
    "include_str",
    "env",
    "concat",
    "stringify",
    "cfg",
    "compile_error",
    "t",
];
/// Developer tooling (perf harness and its CLI): English by design, never shown to users.
const SKIP_DIRS: &[&str] = &["crates/parla/src/perf/", "crates/parla/src/bin/"];
const SKIP_CALLS: &[&str] = &["expect", "debug_selector", "key_context", "font_family"];
const SKIP_CONSTS: &[&str] = &["KEY_CONTEXT"];

fn is_prose(s: &str) -> bool {
    if !s.chars().any(char::is_alphabetic) {
        return false;
    }
    let chars: Vec<char> = s.chars().collect();
    let ws_between_letters = chars.iter().enumerate().any(|(i, c)| {
        c.is_whitespace()
            && chars[..i].iter().any(|c| c.is_alphabetic())
            && chars[i + 1..].iter().any(|c| c.is_alphabetic())
    });
    let capitalised = chars.len() >= 2 && chars[0].is_uppercase() && chars[1].is_lowercase();
    ws_between_letters || capitalised
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("test")
            || (a.path().is_ident("cfg")
                && matches!(&a.meta, syn::Meta::List(l) if l.tokens.to_string() == "test"))
    })
}

#[derive(Default)]
struct Finder {
    found: Vec<String>,
}

impl Finder {
    fn tokens(&mut self, ts: TokenStream) {
        for tt in ts {
            match tt {
                TokenTree::Group(g) => self.tokens(g.stream()),
                TokenTree::Literal(l) => {
                    if let Ok(syn::Lit::Str(s)) = syn::parse_str::<syn::Lit>(&l.to_string()) {
                        self.check(&s.value());
                    }
                }
                _ => {}
            }
        }
    }

    fn check(&mut self, s: &str) {
        if is_prose(s) {
            self.found.push(s.to_string());
        }
    }
}

impl<'ast> Visit<'ast> for Finder {
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}

    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_mod(self, i);
        }
    }
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_fn(self, i);
        }
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_impl(self, i);
        }
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_impl_item_fn(self, i);
        }
    }
    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        if !is_cfg_test(&i.attrs) && !SKIP_CONSTS.contains(&i.ident.to_string().as_str()) {
            syn::visit::visit_item_const(self, i);
        }
    }
    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        if !is_cfg_test(&i.attrs) && !SKIP_CONSTS.contains(&i.ident.to_string().as_str()) {
            syn::visit::visit_item_static(self, i);
        }
    }
    fn visit_impl_item_const(&mut self, i: &'ast syn::ImplItemConst) {
        if !SKIP_CONSTS.contains(&i.ident.to_string().as_str()) {
            syn::visit::visit_impl_item_const(self, i);
        }
    }
    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        if SKIP_CALLS.contains(&e.method.to_string().as_str()) {
            self.visit_expr(&e.receiver);
            return;
        }
        syn::visit::visit_expr_method_call(self, e);
    }
    fn visit_expr_call(&mut self, e: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*e.func
            && (p
                .path
                .segments
                .last()
                .is_some_and(|s| SKIP_CALLS.contains(&s.ident.to_string().as_str()))
                // `log::info(..)` and friends: log lines are never translated.
                || p.path.segments.iter().rev().skip(1).any(|s| s.ident == "log"))
        {
            return;
        }
        syn::visit::visit_expr_call(self, e);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let name = m.path.segments.last().map(|s| s.ident.to_string());
        if name.is_some_and(|n| SKIP_MACROS.contains(&n.as_str())) {
            return;
        }
        self.tokens(m.tokens.clone());
    }
    fn visit_lit_str(&mut self, s: &'ast syn::LitStr) {
        self.check(&s.value());
    }
}

/// Prose literals of one source text.
fn lint_source(src: &str) -> Vec<String> {
    let file = syn::parse_file(src).expect("parses");
    let mut f = Finder::default();
    f.visit_file(&file);
    f.found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(serde::Deserialize)]
struct Allow {
    path: String,
    literal: String,
    reason: String,
}

#[derive(serde::Deserialize)]
struct AllowFile {
    #[serde(default)]
    allow: Vec<Allow>,
}

/// Problems for the source dirs `dirs` (workspace-relative).
fn lint(dirs: &[&str]) -> Vec<String> {
    let allow: AllowFile =
        toml::from_str(&std::fs::read_to_string(root().join("i18n/allow-literals.toml")).unwrap())
            .unwrap();
    let mut used = vec![false; allow.allow.len()];
    let mut problems = Vec::new();
    for dir in dirs {
        let mut files = Vec::new();
        rust_files(&root().join(dir), &mut files);
        files.sort();
        for f in files {
            let rel = f
                .strip_prefix(root())
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if SKIP_DIRS.iter().any(|d| rel.starts_with(d)) {
                continue;
            }
            let src = std::fs::read_to_string(&f).unwrap();
            for lit in lint_source(&src) {
                match allow
                    .allow
                    .iter()
                    .position(|a| a.path == rel && a.literal == lit)
                {
                    Some(i) => used[i] = true,
                    None => problems.push(format!("{rel}: prose literal {lit:?}")),
                }
            }
        }
    }
    for (a, u) in allow.allow.iter().zip(&used) {
        assert!(!a.reason.trim().is_empty(), "{}: empty reason", a.path);
        if dirs.iter().any(|d| a.path.starts_with(d)) && !u {
            problems.push(format!(
                "{}: stale allow-literals entry {:?}",
                a.path, a.literal
            ));
        }
    }
    problems
}

#[test]
fn flags_prose_and_skips_identifiers() {
    assert_eq!(
        lint_source(r#"fn f() { let a = "Sign in"; let b = "sidebar::Open"; }"#),
        ["Sign in"]
    );
    assert_eq!(
        lint_source(r#"fn f() { x.child("hello there"); y.child("Hello"); z.child("kebab-id"); }"#),
        ["hello there", "Hello"]
    );
}

#[test]
fn skips_exempt_positions() {
    let src = r#"
        #[doc = "Some doc text"]
        const KEY_CONTEXT: &str = "Sidebar Panel";
        fn f() {
            x.font_family("JetBrains Mono");
            x.debug_selector(|| "Some Selector".into());
            x.key_context("Key Context");
            tracing::warn!("Upload failed");
            panic!("Boom happened");
            assert_eq!(a, "Left side");
            y.expect("Must be set");
        }
        #[cfg(test)]
        mod tests { fn g() { let _ = "Test only text"; } }
        actions!(sidebar, [Open]);
    "#;
    assert_eq!(lint_source(src), Vec::<String>::new());
}

#[test]
fn flags_literals_in_macros() {
    assert_eq!(
        lint_source(r#"fn f() { let _ = format!("{n} results"); }"#),
        ["{n} results"]
    );
}

#[test]
fn skips_log_calls() {
    assert_eq!(
        lint_source(
            r#"fn f() { crate::log::error(&format!("start failed: {e}")); log::info("Started now"); }"#
        ),
        Vec::<String>::new()
    );
}

#[test]
fn components_has_no_prose() {
    let problems = lint(&["crates/components/src"]);
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn parla_has_no_prose() {
    let problems = lint(&["crates/parla/src"]);
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
