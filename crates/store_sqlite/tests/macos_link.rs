#![cfg(target_os = "macos")]

#[test]
fn no_openssl_linked() {
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new("otool")
        .arg("-L")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(!text.contains("libssl"), "{text}");
    assert!(!text.contains("libcrypto"), "{text}");
}
