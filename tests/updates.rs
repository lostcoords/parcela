use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::{FromRawFd, OwnedFd},
        unix::fs::PermissionsExt,
    },
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

struct Fixture {
    _root: TempDir,
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    exe: PathBuf,
    fake: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_owned();
        let home = path.join("home");
        let exe = home.join(".local/bin/parcela");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_parcela"), &exe).unwrap();
        // Another test thread's fork can briefly hold the fresh copy open for writing, and Linux
        // refuses to run it until that fork execs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while let Err(error) = Command::new(&exe).arg("--version").output() {
            assert!(
                error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && std::time::Instant::now() < deadline,
                "{error}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let fake = path.join("fake");
        fs::create_dir(&fake).unwrap();
        executable(
            &fake.join("curl"),
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TEST_CURL_LOG"
case "$*" in
  *api.github.com*) test "${TEST_OFFLINE:-}" != 1 || exit 7; printf '{"tag_name":"v9.0.0","draft":false,"prerelease":false}' ;;
  *) while [ "$1" != -o ]; do shift; done
     cp "$TEST_INSTALLER" "$2" ;;
esac
"#,
        );
        executable(
            &path.join("installer"),
            r#"#!/bin/sh
printf '%s' "$PARCELA_STATE_DIR" > "$TEST_INSTALL_STATE"
cp "$TEST_REPLACEMENT" "$HOME/.local/bin/parcela.next"
chmod 755 "$HOME/.local/bin/parcela.next"
mv "$HOME/.local/bin/parcela.next" "$HOME/.local/bin/parcela"
"#,
        );
        executable(
            &path.join("replacement"),
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo parcela 9.0.0; else printf 'continued:%s:%s\\n' \"$*\" \"$PARCELA_NO_UPDATE\"; fi\n",
        );
        Self {
            state: path.join("state"),
            _root: root,
            root: path,
            home,
            exe,
            fake,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.exe);
        command
            .current_dir(&self.root)
            .arg("--state-dir")
            .arg(&self.state)
            .arg("status")
            .env("HOME", &self.home)
            .env_remove("CARGO_HOME")
            .env("PATH", format!("{}:/usr/bin:/bin", self.fake.display()))
            .env("PARCELA_NO_TRUST", "1")
            .env_remove("PARCELA_NO_UPDATE")
            .env("TEST_CURL_LOG", self.root.join("curl.log"))
            .env("TEST_INSTALLER", self.root.join("installer"))
            .env("TEST_REPLACEMENT", self.root.join("replacement"))
            .env("TEST_INSTALL_STATE", self.root.join("install-state"));
        command
    }
    fn terminal(&self, command: Command, reply: &str) -> (i32, String) {
        interactive(command, reply)
    }
}
fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
fn interactive(mut command: Command, reply: &str) -> (i32, String) {
    let (mut primary, mut secondary) = (0, 0);
    // SAFETY: openpty writes two new descriptors; the optional name and terminal settings are null.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut primary,
                &mut secondary,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    // SAFETY: the descriptors are fresh and this helper takes ownership exactly once.
    let (mut reader, writer) = unsafe {
        (
            fs::File::from_raw_fd(primary),
            OwnedFd::from_raw_fd(secondary),
        )
    };
    let mut child = command
        .stdin(Stdio::from(writer.try_clone().unwrap()))
        .stdout(Stdio::from(writer.try_clone().unwrap()))
        .stderr(Stdio::from(writer))
        .spawn()
        .unwrap();
    drop(command);
    reader.write_all(reply.as_bytes()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut output = Vec::new();
    let mut buffer = [0; 4096];
    use std::os::fd::AsRawFd;
    loop {
        let mut poll = libc::pollfd {
            fd: reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll has one initialized, valid pollfd and retains no pointer.
        let ready = unsafe { libc::poll(&mut poll, 1, 100) };
        if ready > 0 {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(size) => output.extend_from_slice(&buffer[..size]),
            }
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            panic!(
                "interactive CLI timed out: {}",
                String::from_utf8_lossy(&output)
            );
        }
    }
    (
        child.wait().unwrap().code().unwrap(),
        String::from_utf8(output).unwrap().replace("\r\n", "\n"),
    )
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn declined_update_continues_the_command_and_does_not_prompt_again_that_day() {
    let f = Fixture::new();
    let (code, text) = f.terminal(f.command(), "n\n");
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("9.0.0 is available"), "{text}");
    assert!(text.contains("No environments registered"), "{text}");
    let (code, text) = f.terminal(f.command(), "n\n");
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("Update now?"));
    assert_eq!(
        fs::read_to_string(f.root.join("curl.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn accepted_update_uses_the_selected_state_and_resumes_with_the_new_binary() {
    let f = Fixture::new();
    let (code, text) = f.terminal(f.command(), "y\n");
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains(&format!(
            "continued:--state-dir {} status:1",
            f.state.display()
        )),
        "{text}"
    );
    assert_eq!(
        fs::read_to_string(f.root.join("install-state")).unwrap(),
        f.state.to_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(f.root.join("curl.log"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn piped_commands_and_the_opt_out_never_check_for_updates() {
    let f = Fixture::new();
    assert!(success(f.command().output().unwrap()).contains("workspaces"));
    assert!(!f.state.exists());
    let mut command = f.command();
    command.env("PARCELA_NO_UPDATE", "1");
    let (code, text) = f.terminal(command, "n\n");
    assert_eq!(code, 0, "{text}");
    assert!(!f.root.join("curl.log").exists());
    assert!(!f.state.exists());
}

#[test]
fn an_offline_release_check_never_blocks_the_requested_command() {
    let f = Fixture::new();
    let mut command = f.command();
    command.env("TEST_OFFLINE", "1");
    let (code, text) = f.terminal(command, "n\n");
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("Update now?"));
    assert!(text.contains("No environments registered"));
}

#[test]
fn explicit_json_never_prompts_even_on_a_terminal() {
    let f = Fixture::new();
    let mut command = f.command();
    command.arg("--json");
    let (code, text) = f.terminal(command, "n\n");
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("workspaces"));
    assert!(!text.contains("Update now?"));
    assert!(!f.root.join("curl.log").exists());
    assert!(!f.state.exists());
}

#[test]
fn a_stale_installer_cannot_claim_to_have_installed_the_offered_release() {
    let f = Fixture::new();
    executable(
        &f.root.join("replacement"),
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo parcela 0.0.1; else echo should-not-run; fi\n",
    );
    let (code, text) = f.terminal(f.command(), "y\n");
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("expected at least 9.0.0"), "{text}");
    assert!(!text.contains("should-not-run"));
}
