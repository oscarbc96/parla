//! Catalog completeness checks, shared by `tests/catalogs.rs` and its fixtures.
//! Behind the `check` feature so the app does not link the Fluent parser.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fluent_syntax::ast::{
    CallArguments, Entry, Expression, InlineExpression, Pattern, PatternElement, Resource,
};
use regex::Regex;

use crate::LocaleSet;

/// What the checks need to know about one message or term.
#[derive(Default)]
struct Item {
    vars: BTreeSet<String>,
    attrs: BTreeMap<String, BTreeSet<String>>,
}

type Items = BTreeMap<String, Item>;

/// Checks every crate under `<root>/crates` that has `i18n/en.ftl`, with the
/// locales of `<root>/i18n/locales.toml`. Returns one line per problem.
pub fn check_workspace(root: &Path) -> Vec<String> {
    let toml = match std::fs::read_to_string(root.join("i18n/locales.toml")) {
        Ok(t) => t,
        Err(e) => return vec![format!("i18n/locales.toml: {e}")],
    };
    let locales = match LocaleSet::parse(&toml) {
        Ok(l) => l,
        Err(e) => return vec![format!("i18n/locales.toml: {e}")],
    };
    let mut crates = Vec::new();
    find_crates(&root.join("crates"), &mut crates);
    crates.sort();
    let mut errors = Vec::new();
    for dir in crates {
        let rel = dir.strip_prefix(root).unwrap_or(&dir).display().to_string();
        errors.extend(
            check_crate(&dir, &locales)
                .into_iter()
                .map(|e| format!("{rel}/{e}")),
        );
    }
    errors
}

fn find_crates(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        if name == "target" || name == "node_modules" || name == "fixtures" {
            continue;
        }
        if name == "i18n" && path.join("en.ftl").is_file() {
            if let Some(parent) = path.parent() {
                out.push(parent.to_path_buf());
            }
        } else {
            find_crates(&path, out);
        }
    }
}

/// Checks one crate directory holding `i18n/*.ftl` and `src/`. Lines are
/// relative to `crate_dir`.
pub fn check_crate(crate_dir: &Path, locales: &LocaleSet) -> Vec<String> {
    let mut errors = Vec::new();
    let i18n_dir = crate_dir.join("i18n");
    let source = locales.source.to_string();
    let codes: BTreeSet<String> = locales.locales.iter().map(|l| l.code.to_string()).collect();

    // 1. Files match the locale list.
    if let Ok(rd) = std::fs::read_dir(&i18n_dir) {
        let mut stray: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".ftl").map(str::to_string)
            })
            .filter(|c| !codes.contains(c))
            .collect();
        stray.sort();
        for code in stray {
            errors.push(format!(
                "i18n/{code}.ftl: locale is not listed in locales.toml"
            ));
        }
    }

    let en_text = match std::fs::read_to_string(i18n_dir.join(format!("{source}.ftl"))) {
        Ok(t) => t,
        Err(_) => {
            errors.push(format!("i18n/{source}.ftl: file is missing"));
            return errors;
        }
    };
    let en = parse(&format!("i18n/{source}.ftl"), &en_text, &mut errors);

    for code in codes.iter().filter(|c| **c != source) {
        let file = format!("i18n/{code}.ftl");
        let Ok(text) = std::fs::read_to_string(i18n_dir.join(format!("{code}.ftl"))) else {
            errors.push(format!("{file}: file is missing"));
            continue;
        };
        if let (Some(tr), Some(en)) = (parse(&file, &text, &mut errors), &en) {
            compare(&file, en, &tr, &mut errors);
        }
    }

    // 5. `t!` ids exist in en.ftl; en.ftl ids are used.
    if let Some(en) = &en {
        let used = scan_t_ids(&crate_dir.join("src"), crate_dir, &mut errors, en);
        let ok = unused_ok(&en_text);
        for id in en.keys().filter(|id| !id.starts_with('-')) {
            if !used.contains(id) && !ok.contains(id) {
                errors.push(format!(
                    "i18n/{source}.ftl: message \"{id}\" is never used (add it to `# unused-ok:` if intended)"
                ));
            }
        }
    }
    errors
}

fn parse(file: &str, text: &str, errors: &mut Vec<String>) -> Option<Items> {
    let res = match fluent_syntax::parser::parse(text) {
        Ok(r) => r,
        Err((_, errs)) => {
            for e in errs {
                let line = text[..e.pos.start.min(text.len())].matches('\n').count() + 1;
                errors.push(format!("{file}: parse error at line {line}: {}", e.kind));
            }
            return None;
        }
    };
    Some(collect(&res))
}

fn collect(res: &Resource<&str>) -> Items {
    let mut items = Items::new();
    for entry in &res.body {
        let (id, value, attributes) = match entry {
            Entry::Message(m) => (m.id.name.to_string(), &m.value, &m.attributes),
            Entry::Term(t) => (
                format!("-{}", t.id.name),
                &Some(t.value.clone()),
                &t.attributes,
            ),
            _ => continue,
        };
        let mut item = Item::default();
        if let Some(p) = value {
            pattern_vars(p, &mut item.vars);
        }
        for a in attributes {
            let mut vars = BTreeSet::new();
            pattern_vars(&a.value, &mut vars);
            item.attrs.insert(a.id.name.to_string(), vars);
        }
        items.insert(id, item);
    }
    items
}

fn pattern_vars(p: &Pattern<&str>, out: &mut BTreeSet<String>) {
    for el in &p.elements {
        if let PatternElement::Placeable { expression } = el {
            expr_vars(expression, out);
        }
    }
}

fn expr_vars(e: &Expression<&str>, out: &mut BTreeSet<String>) {
    match e {
        Expression::Inline(i) => inline_vars(i, out),
        Expression::Select { selector, variants } => {
            inline_vars(selector, out);
            for v in variants {
                pattern_vars(&v.value, out);
            }
        }
    }
}

fn inline_vars(i: &InlineExpression<&str>, out: &mut BTreeSet<String>) {
    match i {
        InlineExpression::VariableReference { id } => {
            out.insert(id.name.to_string());
        }
        InlineExpression::Placeable { expression } => expr_vars(expression, out),
        InlineExpression::FunctionReference { arguments, .. } => call_vars(arguments, out),
        InlineExpression::TermReference {
            arguments: Some(arguments),
            ..
        } => call_vars(arguments, out),
        _ => {}
    }
}

fn call_vars(a: &CallArguments<&str>, out: &mut BTreeSet<String>) {
    for p in &a.positional {
        inline_vars(p, out);
    }
    for n in &a.named {
        inline_vars(&n.value, out);
    }
}

/// 3 and 4: same ids and attributes as the source; no invented variables.
fn compare(file: &str, en: &Items, tr: &Items, errors: &mut Vec<String>) {
    for id in en.keys().filter(|id| !tr.contains_key(*id)) {
        errors.push(format!("{file}: missing {} \"{id}\"", kind(id)));
    }
    for (id, t) in tr {
        let Some(s) = en.get(id) else {
            errors.push(format!("{file}: {} \"{id}\" is not in en.ftl", kind(id)));
            continue;
        };
        for a in s.attrs.keys().filter(|a| !t.attrs.contains_key(*a)) {
            errors.push(format!(
                "{file}: {} \"{id}\" is missing attribute \".{a}\"",
                kind(id)
            ));
        }
        for a in t.attrs.keys().filter(|a| !s.attrs.contains_key(*a)) {
            errors.push(format!(
                "{file}: {} \"{id}\" has attribute \".{a}\" not in en.ftl",
                kind(id)
            ));
        }
        let mut allowed = s.vars.clone();
        for v in s.attrs.values() {
            allowed.extend(v.iter().cloned());
        }
        let mut used = t.vars.clone();
        for v in t.attrs.values() {
            used.extend(v.iter().cloned());
        }
        for v in used.difference(&allowed) {
            errors.push(format!(
                "{file}: {} \"{id}\" uses unknown variable \"${v}\"",
                kind(id)
            ));
        }
    }
}

fn kind(id: &str) -> &'static str {
    if id.starts_with('-') {
        "term"
    } else {
        "message"
    }
}

fn unused_ok(en_text: &str) -> BTreeSet<String> {
    en_text
        .lines()
        .filter_map(|l| l.trim_start_matches('#').trim().strip_prefix("unused-ok:"))
        .flat_map(|rest| rest.split_whitespace().map(str::to_string))
        .collect()
}

/// Literal ids passed to `t!` under `src`; reports ids missing from `en`.
fn scan_t_ids(
    src: &Path,
    crate_dir: &Path,
    errors: &mut Vec<String>,
    en: &Items,
) -> BTreeSet<String> {
    let re = Regex::new(r#"\bt!\(\s*[^,()]+,\s*"([^"]+)""#).expect("valid regex");
    let mut files = Vec::new();
    rs_files(src, &mut files);
    files.sort();
    let mut used = BTreeSet::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        // Doc and line comments show example ids; only code counts.
        let text: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let rel = f
            .strip_prefix(crate_dir)
            .unwrap_or(&f)
            .display()
            .to_string();
        for cap in re.captures_iter(&text) {
            let id = cap[1].to_string();
            if !en.contains_key(&id) {
                errors.push(format!("{rel}: t! id \"{id}\" is not in en.ftl"));
            }
            used.insert(id);
        }
    }
    used
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}
