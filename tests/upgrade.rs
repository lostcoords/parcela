use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

/// Runs a copy of parcela placed at `relative` under a fresh HOME, with a fake curl on PATH that
/// downloads an install.sh which only prints its arguments.
fn upgrade(relative: &str, args: &[&str]) -> Output {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let exe = root.path().join(relative.replace("~", "home"));
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_parcela"), &exe).unwrap();
    let fake = root.path().join("fake");
    std::fs::create_dir_all(&fake).unwrap();
    let curl = fake.join("curl");
    std::fs::write(
        &curl,
        "#!/bin/sh\nwhile [ \"$1\" != -o ]; do shift; done\nprintf 'echo install.sh \"$@\"\\n' > \"$2\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", fake.display());
    // Another test thread's fork can briefly hold the fresh copy open for writing, and Linux then
    // refuses to run it.
    for _ in 0..50 {
        match Command::new(&exe)
            .arg("upgrade")
            .args(args)
            .env("HOME", &home)
            .env("PATH", &path)
            .env_remove("CARGO_HOME")
            .output()
        {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            output => return output.unwrap(),
        }
    }
    panic!("the copied parcela stayed busy");
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr)
}

#[test]
fn reruns_install_sh_for_an_install_sh_install() {
    let output = upgrade("~/.local/bin/parcela", &["--nightly"]);
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "install.sh --nightly\n"
    );
    // The test binary is a build of main, and still gets the latest release.
    let output = upgrade("~/.local/bin/parcela", &[]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "install.sh\n");
}

#[test]
fn names_the_package_manager_for_other_installs() {
    for (place, expected) in [
        (
            "brew/Cellar/parcela/0.4.0/bin/parcela",
            "run `brew upgrade parcela`",
        ),
        ("~/.cargo/bin/parcela", "run `cargo install --git"),
        ("usr/local/bin/parcela", "was not installed by install.sh"),
    ] {
        let output = upgrade(place, &[]);
        assert_eq!(output.status.code(), Some(1), "{place}");
        assert!(
            text(&output).contains(expected),
            "{place}: {}",
            text(&output)
        );
        assert!(
            !text(&output).contains("install.sh \n"),
            "{place} ran the installer"
        );
    }
}
