//! Pins invariant 7: no provider crate, type or name leaks into the core.
//!
//! Provider code lives under `crates/providers/<codename>/`. Core crates (every
//! other crate, `recon` and `redact` included, except the `parla` binary) neither depend
//! on a provider crate nor name a provider in their sources or tests.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Crates allowed to depend on provider crates (the registry only).
const WIRING: [&str; 1] = ["parla"];
/// Files allowed to name a provider (paths relative to `crates/`).
const REGISTRY_FILES: [&str; 1] = ["parla/src/providers.rs"];

fn workspace_crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .to_path_buf()
}

fn metadata() -> serde_json::Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata runs");
    assert!(out.status.success(), "cargo metadata failed");
    serde_json::from_slice(&out.stdout).expect("json")
}

fn is_provider_manifest(manifest: &str) -> bool {
    manifest.replace('\\', "/").contains("/crates/providers/")
}

#[test]
fn core_depends_on_no_provider() {
    let meta = metadata();
    let packages = meta["packages"].as_array().expect("packages");
    let provider_names: Vec<&str> = packages
        .iter()
        .filter(|p| is_provider_manifest(p["manifest_path"].as_str().unwrap()))
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(
        !provider_names.is_empty(),
        "no provider crates under crates/providers/"
    );
    let mut seen_domain = false;
    for pkg in packages {
        let name = pkg["name"].as_str().unwrap();
        if is_provider_manifest(pkg["manifest_path"].as_str().unwrap()) || WIRING.contains(&name) {
            continue;
        }
        seen_domain |= name == "domain";
        for dep in pkg["dependencies"].as_array().expect("deps") {
            let d = dep["name"].as_str().unwrap();
            assert!(
                !provider_names.contains(&d),
                "core crate {name} must not depend on provider crate {d}"
            );
        }
    }
    assert!(seen_domain, "domain package not found in metadata");
}

fn rust_and_data_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_and_data_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn core_names_no_provider() {
    // The unofficial notice lives only in the provider catalog, so nothing in
    // core (sources, tests or `i18n/` catalogs) is exempt.
    let crates = workspace_crates();
    // Built piecewise so this file does not match itself.
    let brand = ["sl", "ack"].concat();
    let mut needles = vec![brand.clone()];
    let mut codenames = 0;
    for entry in std::fs::read_dir(crates.join("providers")).expect("crates/providers") {
        let entry = entry.unwrap();
        if entry.path().is_dir() {
            codenames += 1;
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !needles.contains(&name) {
                needles.push(name);
            }
        }
    }
    // The provider folder is named after the brand, so the codename is covered by
    // the brand needle; require the folder to exist so the scan cannot pass vacuously.
    assert!(codenames > 0, "no provider codename found");
    assert!(
        crates.join("providers").join(&brand).is_dir(),
        "expected the provider folder crates/providers/{brand}"
    );
    let mut checked = 0;
    for entry in std::fs::read_dir(&crates).unwrap() {
        let krate = entry.unwrap().path();
        if !krate.is_dir() || krate.file_name().is_some_and(|n| n == "providers") {
            continue;
        }
        let mut files = Vec::new();
        rust_and_data_files(&krate.join("src"), &mut files);
        rust_and_data_files(&krate.join("tests"), &mut files);
        rust_and_data_files(&krate.join("i18n"), &mut files);
        for file in files {
            let rel = file
                .strip_prefix(&crates)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if REGISTRY_FILES.contains(&rel.as_str()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            checked += 1;
            let lower = text.to_lowercase();
            for needle in &needles {
                assert!(
                    !lower.contains(needle.as_str()),
                    "{rel} names a provider: {needle}"
                );
            }
        }
    }
    assert!(checked > 10, "scanned only {checked} files");
}

/// Core catalogs (`crates/<core>/i18n/*.ftl`) name no provider: provider text,
/// including the unofficial notice, lives in the provider's own catalog.
#[test]
fn core_catalogs_name_no_provider() {
    let crates = workspace_crates();
    let brand = ["sl", "ack"].concat();
    let mut seen = 0;
    for entry in std::fs::read_dir(&crates).unwrap() {
        let krate = entry.unwrap().path();
        if !krate.is_dir() || krate.file_name().is_some_and(|n| n == "providers") {
            continue;
        }
        let mut files = Vec::new();
        rust_and_data_files(&krate.join("i18n"), &mut files);
        for file in files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == "ftl"))
        {
            seen += 1;
            let text = std::fs::read_to_string(file).unwrap().to_lowercase();
            assert!(
                !text.contains(&brand),
                "{} names a provider",
                file.display()
            );
        }
    }
    // Core crates with catalogs: components, domain, i18n, parla, three locales each.
    assert!(seen >= 12, "scanned only {seen} catalog files");
}
