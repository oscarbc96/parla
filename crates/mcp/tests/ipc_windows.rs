//! Windows only: named pipe first-instance and owner-only descriptor.
#![cfg(windows)]

use mcp::ipc::{IpcError, IpcServer, platform::owner_only_sddl};

fn pipe(tag: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        r"\\.\pipe\parla-mcp-test-{}-{tag}",
        std::process::id()
    ))
}

#[tokio::test]
async fn pipe_rejects_second_first_instance() {
    let p = pipe("first");
    let _a = IpcServer::bind(&p).unwrap();
    let err = IpcServer::bind(&p).err().unwrap();
    assert!(matches!(err, IpcError::InUse), "{err:?}");
}

#[tokio::test]
async fn pipe_security_descriptor_is_owner_only() {
    // A protected DACL with one allow entry for the current user SID, nobody else.
    let sddl = owner_only_sddl("S-1-5-21-1-2-3-1001");
    assert_eq!(sddl, "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)");
    // The listener builds its descriptor from the real SID and accepts the same user.
    let p = pipe("sd");
    let server = IpcServer::bind(&p).unwrap().run(|_| {});
    let _c = mcp::ipc::connect(&p, "show").await.unwrap();
    server.shutdown();
}
