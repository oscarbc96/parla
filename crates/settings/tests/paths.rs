use std::path::PathBuf;

#[test]
fn data_and_cache_dirs_honour_env() {
    // Single test: it mutates process env.
    unsafe {
        std::env::set_var("PARLA_DATA_DIR", "/tmp/parla-data-x");
        std::env::set_var("PARLA_CACHE_DIR", "/tmp/parla-cache-x");
    }
    assert_eq!(
        settings::paths::data_dir(),
        PathBuf::from("/tmp/parla-data-x")
    );
    assert_eq!(
        settings::paths::cache_dir(),
        PathBuf::from("/tmp/parla-cache-x")
    );
    unsafe {
        std::env::remove_var("PARLA_DATA_DIR");
        std::env::remove_var("PARLA_CACHE_DIR");
    }
    assert!(settings::paths::data_dir().ends_with("parla"));
    assert!(settings::paths::cache_dir().ends_with("parla"));
}
