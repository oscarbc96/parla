//! Feeds arbitrary bytes to the `.parlaplugin` unpacker. It must never
//! panic, never create anything outside its temp root, keep everything it
//! writes inside one staging directory, and leave nothing behind when it
//! refuses a package.

#![no_main]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use libfuzzer_sys::fuzz_target;
use plugin_host::package::{Package, PackageSource};

/// Names in the system temp directory, where a stray write would land.
fn temp_names() -> BTreeSet<PathBuf> {
    fs::read_dir(std::env::temp_dir())
        .expect("read temp dir")
        .map(|e| e.expect("temp entry").path())
        .collect()
}

/// Every path under `dir`, without following links.
fn tree(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).expect("read dir") {
        let p = e.expect("entry").path();
        let ft = fs::symlink_metadata(&p).expect("lstat").file_type();
        assert!(!ft.is_symlink(), "symlink created: {}", p.display());
        out.push(p.clone());
        if ft.is_dir() {
            tree(&p, out);
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let root = tempfile::tempdir().expect("temp dir");
    let staging = root.path().join("staging");
    let cwd = root.path().join("cwd");
    fs::create_dir(&staging).expect("staging dir");
    fs::create_dir(&cwd).expect("cwd dir");
    // A relative write would land here, inside the root, and be caught.
    std::env::set_current_dir(&cwd).expect("chdir");
    let before = temp_names();

    let result = Package::open_bytes(data, PackageSource::File, &staging);

    assert_eq!(before, temp_names(), "files created in the temp directory");
    let mut all = Vec::new();
    tree(root.path(), &mut all);
    let kept = result.as_ref().ok().map(|s| s.dir.path().to_path_buf());
    for p in &all {
        let allowed = *p == staging || *p == cwd || kept.as_ref().is_some_and(|k| p.starts_with(k));
        assert!(allowed, "created outside the staging dir: {}", p.display());
    }
    if result.is_err() {
        assert_eq!(fs::read_dir(&staging).expect("read staging").count(), 0);
    }
});
