//! `.parlaplugin` packages: unpacker with limits, and the packer.
//!
//! A package is a gzip tar with a flat, fixed layout. [`Package::open`]
//! hashes the file, checks the hash against the index for registry
//! packages, then reads every tar entry in one pass. Each entry is checked
//! before any of its bytes are written: type (only regular files and
//! directories), name (ASCII `[A-Za-z0-9._-]` components, no `.`, `..`,
//! empty, hidden or Windows reserved components, no absolute paths), place
//! in the layout, case-insensitive uniqueness, and size. Files land only
//! inside a fresh staging directory, opened with `create_new`, so nothing
//! outside it is ever touched and nothing is overwritten. The decompressed
//! stream itself is capped, so a gzip bomb or a huge tar extension header
//! stops at a few megabytes. Any failure drops the staging directory.
//!
//! Names are ASCII only, so NFC and NFD spellings, and macOS's
//! case-insensitive file system, can never fold two entries onto one file.

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::rc::Rc;

use flate2::Compression;
use flate2::bufread::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::manifest::{self, Manifest, ManifestErrors, ManifestSource};

/// Package limits.
pub mod limits {
    /// The package file, and the sum of its unpacked files.
    pub const MAX_PACKAGE_BYTES: u64 = 5 * 1024 * 1024;
    /// `plugin.wasm`.
    pub const MAX_WASM_BYTES: u64 = 4 * 1024 * 1024;
    /// Every other file.
    pub const MAX_ASSET_BYTES: u64 = 512 * 1024;
    /// Regular files in a package.
    pub const MAX_FILES: usize = 200;
    /// Directory entries (`i18n`, `themes`, `assets`, `icons` and one per
    /// icon pack).
    pub const MAX_DIRS: usize = 4 + crate::manifest::limits::MAX_CONTRIBUTIONS;
    /// Bytes in one path.
    pub const MAX_PATH_BYTES: usize = 200;
    /// Bytes in one path component.
    pub const MAX_COMPONENT_BYTES: usize = 100;
    /// Decompressed tar stream: content plus room for headers, long-name
    /// records and padding (about 2.5 KiB per entry) and the end records.
    pub const MAX_STREAM_BYTES: u64 =
        MAX_PACKAGE_BYTES + (MAX_FILES + MAX_DIRS) as u64 * 2560 + 20 * 1024;
}

use limits::*;

/// Where a package comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageSource {
    /// From the plugin index, which gives the expected SHA-256. Needs
    /// `LICENSE` and `icon.svg`.
    Registry { sha256: [u8; 32] },
    /// "Install from file…": unverified.
    File,
}

/// Why a package was refused. Nothing was installed.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("could not read the package: {0}")]
    Io(#[from] io::Error),
    #[error("the package does not match the checksum in the plugin index")]
    HashMismatch,
    #[error("{what} is over the {limit} byte limit")]
    TooLarge { what: String, limit: u64 },
    #[error("the package has more than {MAX_FILES} files")]
    TooManyFiles,
    #[error("the package has more than {MAX_DIRS} directories")]
    TooManyDirs,
    #[error("`{path}`: {reason}")]
    BadPath { path: String, reason: &'static str },
    #[error("`{path}`: only regular files and directories are allowed")]
    EntryType { path: String },
    #[error("`{path}` appears twice (names are compared ignoring case)")]
    Duplicate { path: String },
    #[error("`{path}` is not part of the package layout")]
    UnknownEntry { path: String },
    #[error("the package has no `{0}`")]
    Missing(&'static str),
    #[error("a data plugin must not contain `{0}`")]
    Unexpected(&'static str),
    #[error("`{path}` is named in plugin.toml but not in the package")]
    MissingDeclared { path: String },
    #[error("plugin.wasm is not a WebAssembly file")]
    NotWasm,
    #[error("not a valid .parlaplugin: {0}")]
    Malformed(String),
    #[error("plugin.toml: {0}")]
    Manifest(ManifestErrors),
}

/// A package unpacked into a staging directory, verified and with a valid
/// manifest. Dropping it removes the directory; the installer moves it.
#[derive(Debug)]
pub struct Staged {
    pub dir: TempDir,
    pub manifest: Manifest,
    /// SHA-256 of the package file.
    pub sha256: [u8; 32],
}

/// The `.parlaplugin` unpacker.
pub struct Package;

impl Package {
    /// Reads, verifies and unpacks the package at `path` into a new
    /// directory under `staging_in`, which should be on the same file
    /// system as the plugins directory so the install is a rename.
    pub fn open(
        path: &Path,
        source: PackageSource,
        staging_in: &Path,
    ) -> Result<Staged, PackageError> {
        let file = fs::File::open(path)?;
        if file.metadata()?.len() > MAX_PACKAGE_BYTES {
            return Err(too_large("the package", MAX_PACKAGE_BYTES));
        }
        let mut bytes = Vec::new();
        file.take(MAX_PACKAGE_BYTES + 1).read_to_end(&mut bytes)?;
        Self::open_bytes(&bytes, source, staging_in)
    }

    /// [`Package::open`] on bytes already in memory.
    pub fn open_bytes(
        bytes: &[u8],
        source: PackageSource,
        staging_in: &Path,
    ) -> Result<Staged, PackageError> {
        if bytes.len() as u64 > MAX_PACKAGE_BYTES {
            return Err(too_large("the package", MAX_PACKAGE_BYTES));
        }
        let sha256: [u8; 32] = Sha256::digest(bytes).into();
        if let PackageSource::Registry { sha256: want } = source
            && want != sha256
        {
            return Err(PackageError::HashMismatch);
        }
        let dir = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(staging_in)?;
        let unpacked = unpack(bytes, dir.path())?;
        let manifest = check(&unpacked, source)?;
        Ok(Staged {
            dir,
            manifest,
            sha256,
        })
    }
}

/// What the unpacker kept besides the files on disk.
struct Unpacked {
    manifest_src: String,
    files: BTreeSet<String>,
}

fn unpack(bytes: &[u8], dest: &Path) -> Result<Unpacked, PackageError> {
    let hit = Rc::new(Cell::new(false));
    let reader = Capped {
        inner: GzDecoder::new(bytes),
        left: MAX_STREAM_BYTES,
        hit: hit.clone(),
    };
    let stream_err = |e: io::Error| {
        if hit.get() {
            too_large("the unpacked package", MAX_STREAM_BYTES)
        } else {
            PackageError::Malformed(e.to_string())
        }
    };
    let mut archive = tar::Archive::new(reader);
    let mut names = Names::default();
    let mut files = 0usize;
    let mut dirs = 0usize;
    let mut total = 0u64;
    let mut manifest_src = None;

    for entry in archive.entries().map_err(stream_err)? {
        let mut entry = entry.map_err(stream_err)?;
        let raw = entry.path_bytes().into_owned();
        let shown = String::from_utf8_lossy(&raw).into_owned();
        let kind = entry.header().entry_type();
        let size = entry.size();
        // A PAX global header only carries archive-wide comments and
        // defaults that `tar` does not apply; its size is bounded by the
        // stream cap.
        if kind == tar::EntryType::XGlobalHeader {
            continue;
        }
        let is_dir = match kind {
            tar::EntryType::Regular => false,
            tar::EntryType::Directory if size == 0 => true,
            _ => return Err(PackageError::EntryType { path: shown }),
        };
        if is_dir {
            dirs += 1;
            if dirs > MAX_DIRS {
                return Err(PackageError::TooManyDirs);
            }
        } else {
            files += 1;
            if files > MAX_FILES {
                return Err(PackageError::TooManyFiles);
            }
        }
        let path = std::str::from_utf8(&raw).map_err(|_| bad(&shown, "not ASCII"))?;
        let path = if is_dir {
            path.strip_suffix('/').unwrap_or(path)
        } else {
            path
        };
        let parts = components(path)?;
        if !in_layout(&parts, is_dir) {
            return Err(PackageError::UnknownEntry { path: shown });
        }
        names.add(&parts, is_dir)?;
        if is_dir {
            fs::create_dir_all(dest.join(path))?;
            continue;
        }

        let cap = if path == "plugin.wasm" {
            MAX_WASM_BYTES
        } else {
            MAX_ASSET_BYTES
        };
        if size > cap {
            return Err(too_large(path, cap));
        }
        total += size;
        if total > MAX_PACKAGE_BYTES {
            return Err(too_large("the unpacked package", MAX_PACKAGE_BYTES));
        }
        let mut data = Vec::with_capacity(size as usize);
        (&mut entry)
            .take(size)
            .read_to_end(&mut data)
            .map_err(stream_err)?;
        if data.len() as u64 != size {
            return Err(PackageError::Malformed(format!("`{path}` is truncated")));
        }
        if path == "plugin.wasm" && !data.starts_with(b"\0asm") {
            return Err(PackageError::NotWasm);
        }
        if path == "plugin.toml" {
            let src = String::from_utf8(data.clone())
                .map_err(|_| PackageError::Malformed("plugin.toml is not UTF-8".into()))?;
            manifest_src = Some(src);
        }
        write_new(&dest.join(path), &data)?;
    }
    // Read to the end so the gzip CRC and length are checked; only zero
    // padding may follow the tar end, and nothing may follow the gzip
    // member.
    let mut rest = archive.into_inner();
    let mut buf = [0u8; 8192];
    loop {
        let n = rest.read(&mut buf).map_err(stream_err)?;
        if n == 0 {
            break;
        }
        if buf[..n].iter().any(|&b| b != 0) {
            return Err(PackageError::Malformed(
                "data after the end of the archive".into(),
            ));
        }
    }
    if !rest.inner.into_inner().is_empty() {
        return Err(PackageError::Malformed(
            "data after the end of the gzip stream".into(),
        ));
    }
    Ok(Unpacked {
        manifest_src: manifest_src.ok_or(PackageError::Missing("plugin.toml"))?,
        files: names.files,
    })
}

/// Rules that need the whole package and its manifest.
fn check(p: &Unpacked, source: PackageSource) -> Result<Manifest, PackageError> {
    let m = manifest::parse(&p.manifest_src, ManifestSource::Package)
        .map_err(PackageError::Manifest)?;
    let has = |f: &str| p.files.contains(f);
    if matches!(source, PackageSource::Registry { .. }) {
        for f in ["LICENSE", "icon.svg"] {
            if !has(f) {
                return Err(PackageError::Missing(f));
            }
        }
    }
    match (m.is_data(), has("plugin.wasm")) {
        (true, true) => return Err(PackageError::Unexpected("plugin.wasm")),
        (false, false) => return Err(PackageError::Missing("plugin.wasm")),
        _ => {}
    }
    if p.files.iter().any(|f| f.starts_with("i18n/")) && !has("i18n/en.ftl") {
        return Err(PackageError::Missing("i18n/en.ftl"));
    }
    for f in m.themes.iter().flat_map(|t| &t.files) {
        if !has(f) {
            return Err(PackageError::MissingDeclared { path: f.clone() });
        }
    }
    for pack in &m.icon_packs {
        let prefix = format!("{}/", pack.dir.trim_end_matches('/'));
        if !p.files.iter().any(|f| f.starts_with(&prefix)) {
            return Err(PackageError::MissingDeclared {
                path: pack.dir.clone(),
            });
        }
    }
    Ok(m)
}

/// Splits and checks a relative path; every rule that keeps a name safe on
/// every file system.
fn components(path: &str) -> Result<Vec<&str>, PackageError> {
    if path.is_empty() {
        return Err(bad(path, "empty path"));
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(bad(path, "path too long"));
    }
    if path.starts_with('/') {
        return Err(bad(path, "absolute path"));
    }
    let parts: Vec<&str> = path.split('/').collect();
    for c in &parts {
        let reason = match *c {
            "" => Some("empty path component"),
            "." | ".." => Some("`.` and `..` are not allowed"),
            c if c.len() > MAX_COMPONENT_BYTES => Some("name too long"),
            c if !c
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-') =>
            {
                Some("names may use only A-Z, a-z, 0-9, `.`, `_` and `-`")
            }
            c if c.starts_with('.') => Some("hidden files are not allowed"),
            c if c.ends_with('.') => Some("names may not end with `.`"),
            c if windows_reserved(c) => Some("reserved file name"),
            _ => None,
        };
        if let Some(r) = reason {
            return Err(bad(path, r));
        }
    }
    Ok(parts)
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM0`-`COM9`, `LPT0`-`LPT9`, with any
/// extension and in any case.
fn windows_reserved(c: &str) -> bool {
    let stem = c.split('.').next().unwrap_or(c).to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        s if s.len() == 4 && (s.starts_with("COM") || s.starts_with("LPT")) => {
            s.as_bytes()[3].is_ascii_digit()
        }
        _ => false,
    }
}

const ROOT_FILES: [&str; 6] = [
    "plugin.toml",
    "plugin.wasm",
    "icon.svg",
    "README.md",
    "CHANGELOG.md",
    "LICENSE",
];
const ROOT_DIRS: [&str; 4] = ["i18n", "themes", "icons", "assets"];

/// The fixed package layout.
fn in_layout(parts: &[&str], is_dir: bool) -> bool {
    let ext = |name: &str, allowed: &[&str]| {
        name.rsplit_once('.').is_some_and(|(stem, e)| {
            !stem.is_empty() && allowed.iter().any(|a| e.eq_ignore_ascii_case(a))
        })
    };
    match (parts, is_dir) {
        ([name], false) => ROOT_FILES.contains(name),
        ([name], true) => ROOT_DIRS.contains(name),
        (["i18n", name], false) => ext(name, &["ftl"]),
        (["themes", name], false) => ext(name, &["json"]),
        (["assets", name], false) => ext(name, &["png", "svg", "webp"]),
        (["icons", _], true) => true,
        (["icons", _, name], false) => ext(name, &["svg"]),
        _ => false,
    }
}

/// Every name seen, compared ignoring ASCII case, so two entries can
/// never land on one file or directory on a case-insensitive disk.
#[derive(Default)]
struct Names {
    /// Lowercased path to its spelling, for files and directories
    /// (including the parents a file implies).
    seen: HashMap<String, (String, bool)>,
    /// Lowercased directories that had their own entry.
    explicit_dirs: HashSet<String>,
    files: BTreeSet<String>,
}

impl Names {
    fn add(&mut self, parts: &[&str], is_dir: bool) -> Result<(), PackageError> {
        let full = parts.join("/");
        let dup = || PackageError::Duplicate { path: full.clone() };
        for i in 1..=parts.len() {
            let path = parts[..i].join("/");
            let last = i == parts.len();
            let dir = !last || is_dir;
            let key = path.to_ascii_lowercase();
            let explicit = last && is_dir;
            match self.seen.get(&key) {
                // A directory seen before, spelled the same: fine once more
                // as a parent, and once as its own entry.
                Some((spelling, true)) if dir && *spelling == path => {
                    if explicit && !self.explicit_dirs.insert(key) {
                        return Err(dup());
                    }
                }
                Some(_) => return Err(dup()),
                None => {
                    if explicit {
                        self.explicit_dirs.insert(key.clone());
                    }
                    self.seen.insert(key, (path, dir));
                }
            }
        }
        if !is_dir {
            self.files.insert(full);
        }
        Ok(())
    }
}

/// A reader that fails once more than `left` bytes come through.
struct Capped<R> {
    inner: R,
    left: u64,
    hit: Rc<Cell<bool>>,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let max = buf
            .len()
            .min(usize::try_from(self.left + 1).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..max])?;
        if n as u64 > self.left {
            self.hit.set(true);
            return Err(io::Error::other("decompressed package over the limit"));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

fn write_new(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o644);
    opts.open(path)?.write_all(data)
}

fn bad(path: &str, reason: &'static str) -> PackageError {
    PackageError::BadPath {
        path: path.to_owned(),
        reason,
    }
}

fn too_large(what: &str, limit: u64) -> PackageError {
    PackageError::TooLarge {
        what: what.to_owned(),
        limit,
    }
}

/// Packs the plugin folder `dir` into `.parlaplugin` bytes (used by the
/// author tooling and by tests). Applies the same name, layout, type and
/// size rules as the unpacker; the output is deterministic (sorted
/// entries, zero times and owners, mode 0644).
pub fn pack(dir: &Path) -> Result<Vec<u8>, PackageError> {
    let mut found = Vec::new();
    walk(dir, "", &mut found)?;
    found.sort();
    let mut names = Names::default();
    let mut files = 0usize;
    let mut total = 0u64;
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    {
        let mut tar = tar::Builder::new(&mut gz);
        for (rel, is_dir) in &found {
            let parts = components(rel)?;
            if !in_layout(&parts, *is_dir) {
                return Err(PackageError::UnknownEntry { path: rel.clone() });
            }
            names.add(&parts, *is_dir)?;
            if *is_dir {
                continue;
            }
            files += 1;
            if files > MAX_FILES {
                return Err(PackageError::TooManyFiles);
            }
            let data = read_no_follow(&dir.join(rel))?;
            let cap = if rel == "plugin.wasm" {
                MAX_WASM_BYTES
            } else {
                MAX_ASSET_BYTES
            };
            if data.len() as u64 > cap {
                return Err(too_large(rel, cap));
            }
            total += data.len() as u64;
            if total > MAX_PACKAGE_BYTES {
                return Err(too_large("the unpacked package", MAX_PACKAGE_BYTES));
            }
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_mode(0o644);
            h.set_mtime(0);
            h.set_uid(0);
            h.set_gid(0);
            h.set_size(data.len() as u64);
            tar.append_data(&mut h, rel, data.as_slice())?;
        }
        tar.finish()?;
    }
    let bytes = gz.finish()?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(too_large("the package", MAX_PACKAGE_BYTES));
    }
    Ok(bytes)
}

/// Reads a regular file without following a symlink that replaced it
/// after the walk.
fn read_no_follow(path: &Path) -> Result<Vec<u8>, PackageError> {
    let mut opts = fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut opts, libc::O_NOFOLLOW);
    #[cfg(not(unix))]
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(not_regular(path));
    }
    let mut f = opts.open(path).map_err(|e| {
        // ELOOP: the last component is a symlink.
        #[cfg(unix)]
        if e.raw_os_error() == Some(libc::ELOOP) {
            return not_regular(path);
        }
        PackageError::Io(e)
    })?;
    if !f.metadata()?.is_file() {
        return Err(not_regular(path));
    }
    let mut data = Vec::new();
    (&mut f).take(MAX_WASM_BYTES + 1).read_to_end(&mut data)?;
    Ok(data)
}

fn not_regular(path: &Path) -> PackageError {
    PackageError::EntryType {
        path: path.display().to_string(),
    }
}

/// Lists `dir` recursively as `(relative path, is_dir)`; refuses links and
/// special files.
fn walk(root: &Path, rel: &str, out: &mut Vec<(String, bool)>) -> Result<(), PackageError> {
    for e in fs::read_dir(root.join(rel))? {
        let e = e?;
        let name = e.file_name();
        let name = name.to_str().ok_or_else(|| {
            bad(
                &name.to_string_lossy(),
                "names may use only A-Z, a-z, 0-9, `.`, `_` and `-`",
            )
        })?;
        let path = if rel.is_empty() {
            name.to_owned()
        } else {
            format!("{rel}/{name}")
        };
        let ft = fs::symlink_metadata(e.path())?.file_type();
        // Editor and VCS litter; any other hidden file is still refused.
        if (ft.is_file() && name == ".DS_Store") || (ft.is_dir() && name == ".git") {
            continue;
        }
        if ft.is_dir() {
            out.push((path.clone(), true));
            walk(root, &path, out)?;
        } else if ft.is_file() {
            out.push((path, false));
        } else {
            return Err(PackageError::EntryType { path });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn read_no_follow_refuses_a_symlink() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("real"), "x").unwrap();
        std::os::unix::fs::symlink(d.path().join("real"), d.path().join("link")).unwrap();
        assert_eq!(read_no_follow(&d.path().join("real")).unwrap(), b"x");
        let e = read_no_follow(&d.path().join("link")).unwrap_err();
        assert!(matches!(e, PackageError::EntryType { .. }), "{e:?}");
    }
}
