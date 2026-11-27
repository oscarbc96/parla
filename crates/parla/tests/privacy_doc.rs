//! Privacy and terms documents state the required v0.1 facts.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(repo_root().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn privacy_names_update_url_and_switch() {
    let privacy = read("PRIVACY.md");
    assert!(
        privacy.contains("https://api.github.com/repos/oscarbc96/parla/releases/latest"),
        "missing update URL:\n{privacy}"
    );
    assert!(
        privacy.contains("updates.check = false"),
        "missing updates.check switch:\n{privacy}"
    );
}

#[test]
fn privacy_and_terms_state_16_plus() {
    let privacy = read("PRIVACY.md");
    let terms = read("TERMS.md");
    for (name, text) in [("PRIVACY.md", &privacy), ("TERMS.md", &terms)] {
        assert!(
            text.contains("16")
                && (text.contains("16 and over")
                    || text.contains("aged 16")
                    || text.contains("16 or older")),
            "{name} missing 16+ statement:\n{text}"
        );
    }
}
