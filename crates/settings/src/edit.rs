//! Comment-preserving edits of JSONC settings text.
//!
//! Only the bytes of the edited value (or one inserted member) change; every
//! comment and all other formatting stay as written.

/// Sets the value at `path` (object keys from the top-level object) to `raw_json`.
/// Missing objects along the path are created. Returns `None` when the text is
/// not an object (for example a syntax error) so the caller never rewrites it blindly.
pub fn update_value(text: &str, path: &[&str], raw_json: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    if text.trim().is_empty() {
        return Some(format!("{}\n", nested(path, raw_json, 1)));
    }
    let start = skip_trivia(text, 0);
    if text.as_bytes().get(start) != Some(&b'{') {
        return None;
    }
    edit_object(text, start, path, raw_json, 1)
}

/// Sets `plugins.<plugin>.config.<key>` in settings `text` to `raw_json`,
/// like [`update_value`]. `secret` is whether the plugin's manifest declares
/// the key `secret`: such a value belongs in the keychain and is refused.
pub fn update_plugin_config(
    text: &str,
    plugin: &str,
    key: &str,
    raw_json: &str,
    secret: bool,
) -> Result<Option<String>, crate::SecretSettingRejected> {
    if secret {
        return Err(crate::SecretSettingRejected { key: key.into() });
    }
    Ok(update_value(
        text,
        &["plugins", plugin, "config", key],
        raw_json,
    ))
}

/// Sets `path` in the settings file at `file` in place: reads it (only
/// `NotFound` counts as empty; any other read error is returned untouched),
/// edits the text with [`update_value`] and writes it back through a temp file
/// plus rename so a reader never sees a half-written file. Returns `Ok(false)`
/// when the file is not an object (nothing is written).
pub fn edit_file(file: &std::path::Path, path: &[&str], raw_json: &str) -> std::io::Result<bool> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let Some(updated) = update_value(&text, path, raw_json) else {
        return Ok(false);
    };
    write_atomic(file, updated.as_bytes()).map(|()| true)
}

/// Removes the member at `path` (object keys from the top-level object) from
/// settings `text`, keeping every comment and all other members. A missing key
/// leaves the text unchanged. Returns `None` when the text is not an object.
pub fn remove_key(text: &str, path: &[&str]) -> Option<String> {
    let (last, parents) = path.split_last()?;
    let mut open = skip_trivia(text, 0);
    if text.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    for key in parents {
        let Some(m) = members(text, open)?.into_iter().find(|m| m.key == *key) else {
            return Some(text.to_string());
        };
        if text.as_bytes()[m.value.0] != b'{' {
            return Some(text.to_string());
        }
        open = m.value.0;
    }
    let mut out = text.to_string();
    // Every copy of a duplicate key goes; each pass re-reads the text.
    loop {
        let list = members(&out, open)?;
        let Some(i) = list.iter().position(|m| m.key == *last) else {
            return Some(out);
        };
        remove_member(&mut out, &list, i, true);
    }
}

/// Removes `path` from the settings file at `file` in place (atomic like
/// [`edit_file`]). `Ok(false)` when the file is missing, not an object or
/// does not have the key (nothing is written).
pub fn remove_key_in_file(file: &std::path::Path, path: &[&str]) -> std::io::Result<bool> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    match remove_key(&text, path) {
        Some(updated) if updated != text => write_atomic(file, updated.as_bytes()).map(|()| true),
        _ => Ok(false),
    }
}

/// Replaces the settings file at `file` with the commented template a first
/// run writes (every key back to its default). Atomic like [`edit_file`];
/// other files in the folder (themes, keymap) are not touched.
pub fn reset_file(file: &std::path::Path) -> std::io::Result<()> {
    write_atomic(file, crate::paths::TEMPLATE.as_bytes())
}

/// Sets `keystrokes` to `action` in the user keymap block of `context`
/// (`None` writes `null`, which unbinds a default binding). The same keys
/// spelled another way (`alt-cmd-h` for `cmd-alt-h`) are replaced. The block
/// is created when missing; every comment and the rest of the text stay as
/// written. Returns `None` when the text is not a keymap array.
pub fn set_user_binding(
    text: &str,
    context: Option<&str>,
    keystrokes: &str,
    action: Option<&str>,
) -> Option<String> {
    let raw = match action {
        Some(a) => quote(a),
        None => "null".to_string(),
    };
    let text = if text.trim().is_empty() { "[]\n" } else { text };
    let open = skip_trivia(text, 0);
    if text.as_bytes().get(open) != Some(&b'[') {
        return None;
    }
    let blocks = elements(text, open)?;
    let canon = crate::keymap::canonical_keys(keystrokes);
    for &(start, _) in &blocks {
        if text.as_bytes()[start] != b'{' || block_context(text, start)?.as_deref() != context {
            continue;
        }
        let mut edited = text.to_string();
        if let Some(bs) = bindings_open(&edited, start)? {
            while remove_one(&mut edited, bs, &|k, _| {
                k != keystrokes && crate::keymap::canonical_keys(k) == canon
            })? {}
        }
        return edit_object(&edited, start, &["bindings", keystrokes], &raw, 2);
    }
    let ctx = context
        .map(|c| format!("\n    \"context\": {},", quote(c)))
        .unwrap_or_default();
    let block = format!(
        "{{{ctx}\n    \"bindings\": {{\n      {}: {raw}\n    }}\n  }}",
        quote(keystrokes)
    );
    let mut out = String::with_capacity(text.len() + block.len() + 8);
    match blocks.last() {
        Some(&(_, end)) => {
            out.push_str(&text[..end]);
            out.push_str(&format!(",\n  {block}"));
            out.push_str(&text[end..]);
        }
        None => {
            // Before the `]`, after anything inside it (a comment stays).
            let close = value_end(text, open)? - 1;
            out.push_str(text[..close].trim_end());
            out.push_str(&format!("\n  {block}\n"));
            out.push_str(&text[close..]);
        }
    }
    Some(out)
}

/// Picks user bindings by `(context, keystrokes, action)`; `action` is
/// `None` for a `null` unbinding.
pub type BindingFilter<'a> = dyn Fn(Option<&str>, &str, Option<&str>) -> bool + 'a;

/// Removes every user binding for which `remove(context, keystrokes, action)`
/// is true (`action` is `None` for a `null` unbinding). Comments, including
/// the one above the next binding and a trailing one, stay; trailing commas
/// stay valid. Returns `None` when the text is not a keymap array.
pub fn remove_user_bindings(text: &str, remove: &BindingFilter<'_>) -> Option<String> {
    if text.trim().is_empty() {
        return Some(String::new());
    }
    let mut text = text.to_string();
    'again: loop {
        let open = skip_trivia(&text, 0);
        if text.as_bytes().get(open) != Some(&b'[') {
            return None;
        }
        for (start, _) in elements(&text, open)? {
            if text.as_bytes()[start] != b'{' {
                continue;
            }
            let context = block_context(&text, start)?;
            let Some(bs) = bindings_open(&text, start)? else {
                continue;
            };
            if remove_one(&mut text, bs, &|k, a| remove(context.as_deref(), k, a))? {
                continue 'again;
            }
        }
        return Some(text);
    }
}

/// Where the `{` of the `bindings` object of the block at `open` is.
fn bindings_open(text: &str, open: usize) -> Option<Option<usize>> {
    let Some(m) = members(text, open)?
        .into_iter()
        .find(|m| m.key == "bindings")
    else {
        return Some(None);
    };
    Some((text.as_bytes()[m.value.0] == b'{').then_some(m.value.0))
}

/// Removes the first member of the object at `open` that `pick(key, value)`
/// accepts (`value` is `None` for `null`). Returns whether one was removed.
fn remove_one(
    text: &mut String,
    open: usize,
    pick: &dyn Fn(&str, Option<&str>) -> bool,
) -> Option<bool> {
    let list = members(text, open)?;
    for (i, m) in list.iter().enumerate() {
        let value: Option<String> = serde_json::from_str(&text[m.value.0..m.value.1]).ok()?;
        if !pick(&m.key, value.as_deref()) {
            continue;
        }
        remove_member(text, &list, i, false);
        return Some(true);
    }
    Some(false)
}

/// Cuts member `i` of `list` out of `text` (with its same-line comment when
/// `with_comment`): its own comma (or the previous
/// member's when it is last), and its line when only indentation is left.
fn remove_member(text: &mut String, list: &[Member], i: usize, with_comment: bool) {
    let m = &list[i];
    let b = text.as_bytes();
    let mut to = m.value.1;
    let mut after = to;
    while b.get(after).is_some_and(u8::is_ascii_whitespace) {
        after += 1;
    }
    // The member's own comma goes with it; a last member without one
    // takes the comma after the previous value instead.
    let mut prev_comma = None;
    if b.get(after) == Some(&b',') {
        to = after + 1;
    } else if let Some(prev) = i.checked_sub(1).map(|p| &list[p]) {
        let c = skip_trivia(text, prev.value.1);
        if b.get(c) == Some(&b',') {
            prev_comma = Some(c);
        }
    }
    // A comment after the member on the same line goes with it.
    let mut c = to;
    while b.get(c).is_some_and(|ch| *ch == b' ' || *ch == b'\t') {
        c += 1;
    }
    let rest = &text[c..];
    if with_comment && rest.starts_with("//") {
        to = rest.find('\n').map_or(text.len(), |n| c + n);
    } else if with_comment && rest.starts_with("/*") {
        let end = rest[2..].find("*/").map(|n| c + n + 4);
        if let Some(end) = end.filter(|e| !text[c..*e].contains('\n')) {
            to = end;
        }
    }
    let from = m.start;
    text.replace_range(from..to, "");
    // Drop the line when nothing but indentation is left on it.
    let line_start = text[..from].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[from..].find('\n').map_or(text.len(), |n| from + n);
    if text[line_start..line_end].trim().is_empty() && line_end < text.len() {
        text.replace_range(line_start..=line_end, "");
    }
    if let Some(c) = prev_comma {
        text.replace_range(c..=c, "");
    }
}

/// Edits the keymap file at `file` with `edit` (missing file reads as empty),
/// writing through a temp file plus rename. `Ok(false)` when `edit` declines.
pub fn edit_keymap_file(
    file: &std::path::Path,
    edit: impl FnOnce(&str) -> Option<String>,
) -> std::io::Result<bool> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let Some(updated) = edit(&text) else {
        return Ok(false);
    };
    write_atomic(file, updated.as_bytes()).map(|()| true)
}

/// Writes `bytes` to `file` through a temp file and rename, so a reader never
/// sees a half-written file. Keeps an existing file's mode and writes through
/// a symlink (a keymap kept in dotfiles stays a link).
pub fn write_atomic(file: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    // Write through a symlink (a keymap kept in dotfiles stays a link).
    let target = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let dir = target.parent().map(std::path::Path::to_path_buf);
    if let Some(dir) = &dir {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let write = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        // Keep the file's mode (a 0600 settings file stays 0600).
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, &target)?;
        // The rename itself is durable once the folder is synced (Unix only;
        // Windows cannot open a folder as a file).
        #[cfg(unix)]
        if let Some(dir) = &dir {
            std::fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    })();
    if write.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write
}

/// The `context` of the block object at `open` (`None` when absent).
fn block_context(text: &str, open: usize) -> Option<Option<String>> {
    match members(text, open)?
        .into_iter()
        .find(|m| m.key == "context")
    {
        Some(m) => serde_json::from_str::<Option<String>>(&text[m.value.0..m.value.1]).ok(),
        None => Some(None),
    }
}

/// The (start, end) spans of the elements of the array whose `[` is at `open`.
fn elements(text: &str, open: usize) -> Option<Vec<(usize, usize)>> {
    let b = text.as_bytes();
    let mut pos = open + 1;
    let mut out = Vec::new();
    loop {
        pos = skip_trivia(text, pos);
        match b.get(pos)? {
            b']' => return Some(out),
            b',' => pos += 1,
            _ => {
                let end = value_end(text, pos)?;
                out.push((pos, end));
                pos = end;
            }
        }
    }
}

fn nested(path: &[&str], raw: &str, depth: usize) -> String {
    let pad = "  ".repeat(depth);
    match path {
        [] => raw.to_string(),
        [last] => format!(
            "{{\n{pad}{}: {raw}\n{}}}",
            quote(last),
            "  ".repeat(depth - 1)
        ),
        [head, rest @ ..] => format!(
            "{{\n{pad}{}: {}\n{}}}",
            quote(head),
            nested(rest, raw, depth + 1),
            "  ".repeat(depth - 1)
        ),
    }
}

fn quote(key: &str) -> String {
    serde_json::to_string(key).unwrap_or_else(|_| format!("\"{key}\""))
}

struct Member {
    key: String,
    /// Where the key's opening quote is.
    start: usize,
    value: (usize, usize),
}

fn edit_object(text: &str, open: usize, path: &[&str], raw: &str, depth: usize) -> Option<String> {
    let members = members(text, open)?;
    let pad = "  ".repeat(depth);
    let Some(m) = members.iter().find(|m| m.key == path[0]) else {
        let insert = if path.len() == 1 {
            format!("{}: {raw}", quote(path[0]))
        } else {
            format!("{}: {}", quote(path[0]), nested(&path[1..], raw, depth + 1))
        };
        let comma = if members.is_empty() { "" } else { "," };
        let mut out = String::with_capacity(text.len() + insert.len() + 8);
        out.push_str(&text[..=open]);
        out.push_str(&format!("\n{pad}{insert}{comma}"));
        out.push_str(&text[open + 1..]);
        return Some(out);
    };
    let (vs, ve) = m.value;
    let replacement = if path.len() == 1 {
        raw.to_string()
    } else if text.as_bytes()[vs] == b'{' {
        let edited = edit_object(text, vs, &path[1..], raw, depth + 1)?;
        // `edited` is the whole text; splice only the object part back.
        let tail = text.len() - ve;
        edited[vs..edited.len() - tail].to_string()
    } else {
        nested(&path[1..], raw, depth + 1)
    };
    Some(format!("{}{replacement}{}", &text[..vs], &text[ve..]))
}

/// Parses the members of the object whose `{` is at `open`.
fn members(text: &str, open: usize) -> Option<Vec<Member>> {
    let b = text.as_bytes();
    let mut pos = open + 1;
    let mut out = Vec::new();
    loop {
        pos = skip_trivia(text, pos);
        match b.get(pos)? {
            b'}' => return Some(out),
            b',' => pos += 1,
            b'"' => {
                let key_start = pos;
                let key_end = string_end(text, pos)?;
                let key: String = serde_json::from_str(&text[pos..key_end]).ok()?;
                pos = skip_trivia(text, key_end);
                if b.get(pos) != Some(&b':') {
                    return None;
                }
                let vs = skip_trivia(text, pos + 1);
                let ve = value_end(text, vs)?;
                out.push(Member {
                    key,
                    start: key_start,
                    value: (vs, ve),
                });
                pos = ve;
            }
            _ => return None,
        }
    }
}

fn skip_trivia(text: &str, mut pos: usize) -> usize {
    let b = text.as_bytes();
    loop {
        match b.get(pos) {
            Some(c) if c.is_ascii_whitespace() => pos += 1,
            Some(b'/') if b.get(pos + 1) == Some(&b'/') => {
                while b.get(pos).is_some_and(|c| *c != b'\n') {
                    pos += 1;
                }
            }
            Some(b'/') if b.get(pos + 1) == Some(&b'*') => {
                pos += 2;
                while pos < b.len() && !(b[pos] == b'*' && b.get(pos + 1) == Some(&b'/')) {
                    pos += 1;
                }
                pos = (pos + 2).min(b.len());
            }
            _ => return pos,
        }
    }
}

/// Index one past the closing quote of the string starting at `pos`.
fn string_end(text: &str, pos: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = pos + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Index one past the value starting at `pos`.
fn value_end(text: &str, pos: usize) -> Option<usize> {
    let b = text.as_bytes();
    match b.get(pos)? {
        b'"' => string_end(text, pos),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = pos;
            while i < b.len() {
                match b[i] {
                    b'"' => i = string_end(text, i)?,
                    b'/' if matches!(b.get(i + 1), Some(b'/' | b'*')) => i = skip_trivia(text, i),
                    b'{' | b'[' => {
                        depth += 1;
                        i += 1;
                    }
                    b'}' | b']' => {
                        depth -= 1;
                        i += 1;
                        if depth == 0 {
                            return Some(i);
                        }
                    }
                    _ => i += 1,
                }
            }
            None
        }
        _ => {
            let mut i = pos;
            while i < b.len()
                && !matches!(b[i], b',' | b'}' | b']' | b'/')
                && !b[i].is_ascii_whitespace()
            {
                i += 1;
            }
            Some(i)
        }
    }
}

#[cfg(test)]
mod edit_file_tests {
    use super::edit_file;

    #[test]
    fn creates_missing_file_and_edits_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("settings.json");
        assert!(edit_file(&f, &["a", "b"], "1").unwrap());
        assert!(edit_file(&f, &["c"], "2").unwrap());
        let t = std::fs::read_to_string(&f).unwrap();
        assert!(t.contains("\"b\": 1") && t.contains("\"c\": 2"), "{t}");
        assert!(!dir.path().join("settings.json.tmp").exists());
    }

    #[test]
    fn unreadable_file_is_not_clobbered() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("settings.json");
        std::fs::write(&f, [0xff, 0xfe, 0x00]).unwrap();
        assert!(edit_file(&f, &["a"], "1").is_err());
        assert_eq!(std::fs::read(&f).unwrap(), vec![0xff, 0xfe, 0x00]);
    }
}
