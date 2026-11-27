#[cfg(not(target_os = "macos"))]
#[test]
fn install_returns_unsupported_off_macos() {
    let result = parla::updates::install::install(
        std::path::Path::new("update.dmg"),
        std::path::Path::new("Parla.app"),
        std::path::Path::new("mount"),
        &parla::updates::install::SystemRunner,
    );

    assert_eq!(
        result,
        Err(parla::updates::install::InstallError::Unsupported)
    );
}

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::{HashMap, VecDeque};
    use std::ffi::OsString;
    use std::io;
    use std::os::unix::process::ExitStatusExt as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::sync::Mutex;

    use parla::updates::install::{
        BUNDLE_ID, InstallError, NotInstallable, Runner, SystemRunner, install, running_bundle,
        swap,
    };
    use tempfile::TempDir;

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<Vec<String>>>,
        outputs: Mutex<HashMap<String, VecDeque<io::Result<Output>>>>,
        create_app: bool,
    }

    impl FakeRunner {
        fn new(create_app: bool) -> Self {
            Self {
                create_app,
                ..Self::default()
            }
        }

        fn script(&self, tool: &str, outputs: Vec<io::Result<Output>>) {
            self.outputs
                .lock()
                .unwrap()
                .insert(tool.to_owned(), outputs.into());
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, argv: &[OsString]) -> io::Result<Output> {
            let call: Vec<String> = argv
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            let result = self
                .outputs
                .lock()
                .unwrap()
                .get_mut(&call[0])
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| output(true, ""));

            if self.create_app
                && call[0] == "/usr/bin/hdiutil"
                && call.get(1).is_some_and(|arg| arg == "attach")
                && result.as_ref().is_ok_and(|out| out.status.success())
            {
                let mount = call
                    .windows(2)
                    .find(|args| args[0] == "-mountpoint")
                    .map(|args| PathBuf::from(&args[1]))
                    .unwrap();
                let contents = mount.join("Parla.app/Contents");
                std::fs::create_dir_all(&contents).unwrap();
                std::fs::write(contents.join("Info.plist"), "").unwrap();
            }

            if call[0] == "/usr/bin/rsync" {
                // The staging copy exists even when rsync then fails halfway.
                let dest = PathBuf::from(call.last().unwrap());
                std::fs::create_dir_all(&dest).unwrap();
                std::fs::write(dest.join("new.txt"), "new").unwrap();
            }

            self.calls.lock().unwrap().push(call);
            result
        }
    }

    fn output(success: bool, stdout: &str) -> io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(if success { 0 } else { 1 << 8 }),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }

    fn failure() -> io::Result<Output> {
        output(false, "")
    }

    struct Fixture {
        _temp: TempDir,
        dmg: PathBuf,
        bundle: PathBuf,
        mount: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let dmg = temp.path().join("update.dmg");
            let bundle = temp.path().join("installed/Parla.app");
            let mount = temp.path().join("mount");
            std::fs::write(&dmg, "dmg").unwrap();
            std::fs::create_dir_all(&bundle).unwrap();
            std::fs::write(bundle.join("old.txt"), "old").unwrap();
            std::fs::create_dir_all(&mount).unwrap();
            Self {
                _temp: temp,
                dmg,
                bundle,
                mount,
            }
        }

        fn app(&self) -> PathBuf {
            self.mount.join("Parla.app")
        }

        /// The staging copy next to the installed bundle.
        fn staged(&self) -> PathBuf {
            let mut name = self.bundle.as_os_str().to_owned();
            name.push(".new");
            PathBuf::from(name)
        }

        fn old_app_untouched(&self) -> bool {
            self.bundle.join("old.txt").exists() && !self.bundle.join("new.txt").exists()
        }

        fn nothing_left_beside_the_bundle(&self) -> bool {
            std::fs::read_dir(self.bundle.parent().unwrap())
                .unwrap()
                .count()
                == 1
        }

        fn install(&self, runner: &dyn Runner) -> Result<(), InstallError> {
            install(&self.dmg, &self.bundle, &self.mount, runner)
        }

        fn attach(&self) -> Vec<String> {
            argv(&[
                "/usr/bin/hdiutil",
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
                path(&self.mount),
                path(&self.dmg),
            ])
        }

        fn codesign(&self) -> Vec<String> {
            argv(&[
                "/usr/bin/codesign",
                "--verify",
                "--deep",
                "--strict",
                path(&self.app()),
            ])
        }

        fn codesign_staged(&self) -> Vec<String> {
            argv(&[
                "/usr/bin/codesign",
                "--verify",
                "--deep",
                "--strict",
                path(&self.staged()),
            ])
        }

        fn plist_buddy(&self) -> Vec<String> {
            argv(&[
                "/usr/libexec/PlistBuddy",
                "-c",
                "Print :CFBundleIdentifier",
                path(&self.app().join("Contents/Info.plist")),
            ])
        }

        fn rsync(&self) -> Vec<String> {
            argv(&[
                "/usr/bin/rsync",
                "-a",
                "--delete",
                &format!("{}/", path(&self.app())),
                &format!("{}/", path(&self.staged())),
            ])
        }

        fn detach(&self, force: bool) -> Vec<String> {
            if force {
                argv(&["/usr/bin/hdiutil", "detach", "-force", path(&self.mount)])
            } else {
                argv(&["/usr/bin/hdiutil", "detach", path(&self.mount)])
            }
        }
    }

    fn path(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    fn script_valid_install(runner: &FakeRunner) {
        runner.script("/usr/bin/hdiutil", vec![output(true, ""), output(true, "")]);
        runner.script(
            "/usr/bin/codesign",
            vec![output(true, ""), output(true, "")],
        );
        runner.script(
            "/usr/libexec/PlistBuddy",
            vec![output(true, &format!("{BUNDLE_ID}\n"))],
        );
        runner.script("/usr/bin/rsync", vec![output(true, "")]);
    }

    #[test]
    fn success_runs_the_exact_command_sequence_and_swaps_the_app() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(true);
        script_valid_install(&runner);

        assert_eq!(fixture.install(&runner), Ok(()));
        assert_eq!(
            runner.calls(),
            vec![
                fixture.attach(),
                fixture.codesign(),
                fixture.plist_buddy(),
                fixture.rsync(),
                fixture.detach(false),
                fixture.codesign_staged(),
            ]
        );
        assert!(fixture.bundle.join("new.txt").exists());
        assert!(!fixture.bundle.join("old.txt").exists());
        assert!(fixture.nothing_left_beside_the_bundle());
    }

    #[test]
    fn attach_failure_runs_nothing_else() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(true);
        runner.script("/usr/bin/hdiutil", vec![failure()]);

        assert_eq!(fixture.install(&runner), Err(InstallError::Attach));
        assert_eq!(runner.calls(), vec![fixture.attach()]);
    }

    #[test]
    fn missing_app_detaches() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(false);
        runner.script("/usr/bin/hdiutil", vec![output(true, ""), output(true, "")]);

        assert_eq!(fixture.install(&runner), Err(InstallError::MissingApp));
        assert_eq!(
            runner.calls(),
            vec![fixture.attach(), fixture.detach(false)]
        );
    }

    #[test]
    fn bad_signature_detaches_without_copying() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(true);
        runner.script("/usr/bin/hdiutil", vec![output(true, ""), output(true, "")]);
        runner.script("/usr/bin/codesign", vec![failure()]);

        assert_eq!(fixture.install(&runner), Err(InstallError::BadSignature));
        assert_eq!(
            runner.calls(),
            vec![fixture.attach(), fixture.codesign(), fixture.detach(false),]
        );
    }

    #[test]
    fn wrong_bundle_id_detaches_without_copying() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(true);
        runner.script("/usr/bin/hdiutil", vec![output(true, ""), output(true, "")]);
        runner.script("/usr/bin/codesign", vec![output(true, "")]);
        runner.script(
            "/usr/libexec/PlistBuddy",
            vec![output(true, "com.example.other\n")],
        );

        assert_eq!(fixture.install(&runner), Err(InstallError::WrongBundleId));
        assert_eq!(
            runner.calls(),
            vec![
                fixture.attach(),
                fixture.codesign(),
                fixture.plist_buddy(),
                fixture.detach(false),
            ]
        );
    }

    #[test]
    fn copy_failure_leaves_the_installed_app_untouched() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(true);
        runner.script("/usr/bin/hdiutil", vec![output(true, ""), output(true, "")]);
        runner.script("/usr/bin/codesign", vec![output(true, "")]);
        runner.script("/usr/libexec/PlistBuddy", vec![output(true, BUNDLE_ID)]);
        runner.script("/usr/bin/rsync", vec![failure()]);

        assert_eq!(fixture.install(&runner), Err(InstallError::Copy));
        assert_eq!(
            runner.calls(),
            vec![
                fixture.attach(),
                fixture.codesign(),
                fixture.plist_buddy(),
                fixture.rsync(),
                fixture.detach(false),
            ]
        );
        assert!(fixture.old_app_untouched());
        assert!(fixture.nothing_left_beside_the_bundle());
    }

    #[test]
    fn a_staged_copy_that_fails_codesign_is_never_swapped_in() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(true);
        script_valid_install(&runner);
        runner.script("/usr/bin/codesign", vec![output(true, ""), failure()]);

        assert_eq!(fixture.install(&runner), Err(InstallError::BadSignature));
        assert!(fixture.old_app_untouched());
        assert!(fixture.nothing_left_beside_the_bundle());
    }

    #[test]
    fn a_stale_staging_copy_is_removed_before_copying() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.staged()).unwrap();
        std::fs::write(fixture.staged().join("stale.txt"), "x").unwrap();
        let runner = FakeRunner::new(true);
        script_valid_install(&runner);

        assert_eq!(fixture.install(&runner), Ok(()));
        assert!(!fixture.bundle.join("stale.txt").exists());
        assert!(fixture.nothing_left_beside_the_bundle());
    }

    #[test]
    fn detach_failure_tries_force() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(true);
        script_valid_install(&runner);
        runner.script(
            "/usr/bin/hdiutil",
            vec![output(true, ""), failure(), output(true, "")],
        );

        assert_eq!(fixture.install(&runner), Ok(()));
        assert_eq!(
            runner.calls(),
            vec![
                fixture.attach(),
                fixture.codesign(),
                fixture.plist_buddy(),
                fixture.rsync(),
                fixture.detach(false),
                fixture.detach(true),
                fixture.codesign_staged(),
            ]
        );
    }

    #[test]
    fn a_failed_detach_after_a_good_copy_still_installs() {
        let fixture = Fixture::new();
        let runner = FakeRunner::new(true);
        script_valid_install(&runner);
        runner.script(
            "/usr/bin/hdiutil",
            vec![output(true, ""), failure(), failure()],
        );

        assert_eq!(fixture.install(&runner), Ok(()));
        assert!(fixture.bundle.join("new.txt").exists());
        assert!(fixture.nothing_left_beside_the_bundle());
    }

    #[test]
    fn swap_replaces_the_bundle_and_cleans_up() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.staged()).unwrap();
        std::fs::write(fixture.staged().join("new.txt"), "new").unwrap();

        assert_eq!(swap(&fixture.staged(), &fixture.bundle), Ok(()));
        assert!(fixture.bundle.join("new.txt").exists());
        assert!(!fixture.bundle.join("old.txt").exists());
        assert!(fixture.nothing_left_beside_the_bundle());
    }

    #[test]
    fn a_failed_swap_restores_the_installed_app() {
        let fixture = Fixture::new();
        // The staging copy is missing: neither the exchange nor the rename works.
        assert_eq!(
            swap(&fixture.staged(), &fixture.bundle),
            Err(InstallError::Swap)
        );
        assert!(fixture.old_app_untouched());
        assert!(fixture.nothing_left_beside_the_bundle());
    }

    #[test]
    fn running_bundle_accepts_an_app_bundle() {
        let exe = Path::new("/Applications/Parla.app/Contents/MacOS/parla");

        assert_eq!(
            running_bundle(exe, &|_| true),
            Ok(PathBuf::from("/Applications/Parla.app"))
        );
    }

    #[test]
    fn running_bundle_rejects_a_non_bundle() {
        assert_eq!(
            running_bundle(Path::new("/tmp/parla"), &|_| true),
            Err(NotInstallable::NotABundle)
        );
    }

    #[test]
    fn running_bundle_rejects_translocation() {
        let exe = Path::new(
            "/private/var/folders/x/AppTranslocation/id/d/Parla.app/Contents/MacOS/parla",
        );

        assert_eq!(
            running_bundle(exe, &|_| true),
            Err(NotInstallable::Translocated)
        );
    }

    #[test]
    fn running_bundle_rejects_mounted_volumes() {
        let exe = Path::new("/Volumes/Parla/Parla.app/Contents/MacOS/parla");

        assert_eq!(
            running_bundle(exe, &|_| true),
            Err(NotInstallable::OnMountedVolume)
        );
    }

    #[test]
    fn running_bundle_rejects_read_only_locations() {
        let exe = Path::new("/Applications/Parla.app/Contents/MacOS/parla");

        assert_eq!(
            running_bundle(exe, &|_| false),
            Err(NotInstallable::ReadOnly)
        );
    }

    #[test]
    fn running_bundle_needs_both_the_bundle_and_its_parent_writable() {
        let exe = Path::new("/Applications/Parla.app/Contents/MacOS/parla");

        assert_eq!(
            running_bundle(exe, &|path| path == Path::new("/Applications")),
            Err(NotInstallable::ReadOnly)
        );
        assert_eq!(
            running_bundle(exe, &|path| path == Path::new("/Applications/Parla.app")),
            Err(NotInstallable::ReadOnly)
        );
        assert_eq!(
            running_bundle(exe, &|_| true),
            Ok(PathBuf::from("/Applications/Parla.app"))
        );
    }

    #[test]
    #[ignore]
    fn real_dmg_installs_over_a_temp_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let source_app = source.join("Parla.app");
        let source_macos = source_app.join("Contents/MacOS");
        std::fs::create_dir_all(&source_macos).unwrap();
        let source_exe = source_macos.join("parla");
        std::fs::copy("/usr/bin/true", &source_exe).unwrap();
        std::fs::write(
            source_app.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>parla</string>
    <key>CFBundleIdentifier</key>
    <string>{BUNDLE_ID}</string>
    <key>CFBundleName</key>
    <string>Parla</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
</dict>
</plist>
"#
            ),
        )
        .unwrap();

        let signed = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&source_app)
            .output()
            .unwrap();
        assert!(
            signed.status.success(),
            "{}",
            String::from_utf8_lossy(&signed.stderr)
        );

        let dmg = temp.path().join("update.dmg");
        let created = Command::new("/usr/bin/hdiutil")
            .arg("create")
            .arg("-srcfolder")
            .arg(&source)
            .args(["-format", "UDZO"])
            .arg(&dmg)
            .output()
            .unwrap();
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );

        let target_app = temp.path().join("target/Parla.app");
        let target_exe = target_app.join("Contents/MacOS/parla");
        std::fs::create_dir_all(target_exe.parent().unwrap()).unwrap();
        std::fs::write(&target_exe, "old executable").unwrap();
        let mount = temp.path().join("mount");
        std::fs::create_dir(&mount).unwrap();

        install(&dmg, &target_app, &mount, &SystemRunner).unwrap();

        assert_eq!(
            std::fs::read(target_exe).unwrap(),
            std::fs::read(source_exe).unwrap()
        );
    }
}
