use parcela::{
    environment::control::Server,
    project::config::Manifest,
    state::registry::{Registry, Role},
    state::sockets,
};
use std::{
    fs,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
};
use tempfile::TempDir;

fn long_state(temp: &TempDir) -> (Registry, PathBuf, String) {
    let state = temp.path().join("s".repeat(80)).join("t".repeat(80));
    assert!(state.as_os_str().len() > 150);
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let manifest: Manifest = Manifest::parse(
        "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
    )
    .unwrap();
    let mut registry = Registry::open(&state, true).unwrap().unwrap();
    let workspace = registry
        .adopt(&manifest, &source, Role::Preview, "auth")
        .unwrap();
    let runtime = registry.runtime_path(&workspace);
    (registry, runtime, workspace.id)
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o777
}

fn base() -> PathBuf {
    sockets::base()
}

fn hashed_directory(socket: &Path, base: &Path) -> String {
    let directory = socket.parent().unwrap();
    assert_eq!(directory.parent().unwrap(), base);
    directory.file_name().unwrap().to_str().unwrap().to_owned()
}

#[test]
fn long_state_directories_bind_short_private_sockets() {
    let temp = TempDir::new_in("/tmp").unwrap();
    let (registry, runtime, id) = long_state(&temp);
    let prefix: String = id.replace('-', "")[..12].to_owned();
    assert!(runtime.join("pc.sock").as_os_str().len() > 150);

    let environment = sockets::environment(&runtime).unwrap();
    let admin = sockets::proxy_admin(registry.state_path()).unwrap();
    assert_eq!(hashed_directory(&environment.control, &base()).len(), 8);
    assert_eq!(
        environment.control.file_name().unwrap().to_str().unwrap(),
        format!("{prefix}-ctl.sock")
    );
    assert_eq!(
        admin,
        environment.control.with_file_name("proxy-admin.sock")
    );

    let server = Server::bind(&runtime).unwrap();
    assert!(environment.control.as_os_str().len() < 100);
    assert_eq!(mode(environment.control.parent().unwrap()), 0o700);
    assert_eq!(mode(&environment.control), 0o600);
    sockets::prepare(&admin).unwrap();
    let listener = UnixListener::bind(&admin).unwrap();
    drop((server, listener));
    assert!(!environment.control.exists());
    fs::remove_file(&admin).unwrap();
    assert_eq!(mode(&base()), 0o700);
    fs::remove_dir(environment.control.parent().unwrap()).unwrap();
    assert_eq!(
        sockets::environment(&runtime).unwrap().control,
        environment.control
    );
}

#[test]
fn shared_or_linked_socket_directories_are_refused() {
    let temp = TempDir::new_in("/tmp").unwrap();
    let (registry, runtime, _) = long_state(&temp);
    let socket = sockets::environment(&runtime).unwrap().control;
    let directory = socket.parent().unwrap().to_owned();

    // The base may not exist yet on a fresh machine.
    parcela::state::private::directory(directory.parent().unwrap()).unwrap();
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o777)).unwrap();
    let error = Server::bind(&runtime).err().unwrap();
    assert!(format!("{error:#}").contains("private directory (700)"));
    assert!(sockets::prepare(&sockets::proxy_admin(registry.state_path()).unwrap()).is_err());
    assert!(!socket.exists());
    assert_eq!(mode(&directory), 0o777);

    fs::remove_dir(&directory).unwrap();
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &directory).unwrap();
    assert!(Server::bind(&runtime).is_err());
    assert!(fs::read_dir(&elsewhere).unwrap().next().is_none());

    fs::remove_file(&directory).unwrap();
}
