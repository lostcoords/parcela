use parcela::{
    config::Manifest,
    control::{self, Action, Server},
    lifecycle, private, process,
    registry::{Registry, Role, SessionStatus},
};
use std::{
    fs,
    os::unix::{fs::symlink, net::UnixListener},
    process::Command,
    thread,
    time::Duration,
};
use tempfile::TempDir;
use uuid::Uuid;

/// A record's children: one `web` process with this identity.
fn web(
    identity: &process::Identity,
) -> std::collections::BTreeMap<String, parcela::supervisor::Child> {
    std::collections::BTreeMap::from([(
        "web".to_owned(),
        parcela::supervisor::Child {
            identity: Some(identity.clone()),
            exit: None,
            restarts: 0,
            stopped: false,
        },
    )])
}

fn control_socket(runtime: &std::path::Path) -> std::path::PathBuf {
    parcela::sockets::environment(runtime).unwrap().control
}

fn fixture() -> (
    TempDir,
    Registry,
    parcela::registry::Workspace,
    parcela::registry::Session,
) {
    let temp = TempDir::new_in("/tmp").unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let manifest: Manifest = Manifest::parse(
        "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
    )
    .unwrap();
    let mut registry = Registry::open(&temp.path().join("state"), true)
        .unwrap()
        .unwrap();
    let workspace = registry
        .adopt(&manifest, &source, Role::Preview, "auth")
        .unwrap();
    let session = registry
        .begin_session(
            &workspace,
            &Uuid::new_v4().to_string(),
            32001,
            "up",
            "https://auth-app.localhost",
        )
        .unwrap();
    (temp, registry, workspace, session)
}

#[test]
fn unavailable_owner_is_unknown_and_inspection_creates_no_runtime() {
    let (_temp, registry, workspace, _session) = fixture();
    let status = lifecycle::status(&registry, &workspace).unwrap();
    assert_eq!(status["state"], "unknown");
    assert_eq!(status["live_health_verified"], false);
    assert!(lifecycle::down(&registry, &workspace).is_err());
    assert!(!registry.runtime_path(&workspace).exists());
}

#[test]
fn down_converges_a_start_that_died_before_saving_its_record() {
    let (_temp, registry, workspace, _session) = fixture();
    let path = registry.runtime_path(&workspace);
    private::directory(&path).unwrap();
    let held = private::Lock::acquire(&path.join("owner.lock")).unwrap();
    assert!(lifecycle::down(&registry, &workspace).is_err());
    drop(held);
    // A child forked by a parallel test can share the lock's file for a moment.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let stopped = loop {
        match lifecycle::down(&registry, &workspace) {
            Ok(stopped) => break stopped,
            Err(error) => assert!(std::time::Instant::now() < deadline, "{error:#}"),
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(stopped["state"], "stopped");
    let current = registry.session(&workspace.id).unwrap().unwrap();
    assert_eq!(current.status, SessionStatus::Stopped);
}

#[test]
fn repeated_down_is_noop_without_runtime_or_process_signaling() {
    let (_temp, mut registry, workspace, session) = fixture();
    registry
        .session_status(&session, SessionStatus::Stopped)
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            lifecycle::down(&registry, &workspace).unwrap()["already_stopped"],
            true
        );
    }
    assert!(!registry.runtime_path(&workspace).exists());
}

#[test]
fn stale_nonce_or_generation_never_reaches_owner_actions() {
    let (_temp, registry, workspace, session) = fixture();
    let path = registry.runtime_path(&workspace);
    let server = Server::bind(&path).unwrap();
    for nonce in [true, false] {
        let mut stale = session.clone();
        if nonce {
            stale.token = Uuid::new_v4().to_string();
        } else {
            stale.generation += 1;
        }
        use std::io::{Read, Write};
        let mut stream = std::os::unix::net::UnixStream::connect(control_socket(&path)).unwrap();
        let request = serde_json::json!({"environment_id": stale.environment_id, "token": stale.token, "generation": stale.generation, "action": {"type": "stop"}});
        writeln!(stream, "{request}").unwrap();
        assert!(server.receive(&session).unwrap().is_none());
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("stale or foreign owner request"));
    }
    let root = path.clone();
    let selected = session.clone();
    let client = thread::spawn(move || control::request(&root, &selected, Action::Status));
    let pending = loop {
        if let Some(pending) = server.receive(&session).unwrap() {
            break pending;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert!(matches!(pending.action, Action::Status));
    pending
        .reply(&session, Ok(serde_json::json!({"verified": true})))
        .unwrap();
    assert_eq!(client.join().unwrap().unwrap()["verified"], true);
    drop(server);
    assert!(!control_socket(&path).exists());
}

#[test]
fn disconnected_caller_retains_complete_stop_and_cannot_fail_owner() {
    use std::{io::Write, os::unix::net::UnixStream};

    let (_temp, registry, workspace, session) = fixture();
    let path = registry.runtime_path(&workspace);
    let server = Server::bind(&path).unwrap();
    let request = serde_json::json!({"environment_id": session.environment_id, "token": session.token, "generation": session.generation, "action": {"type": "stop"}});
    let mut caller = UnixStream::connect(control_socket(&path)).unwrap();
    writeln!(caller, "{request}").unwrap();
    caller.shutdown(std::net::Shutdown::Both).unwrap();
    drop(caller);
    let pending = server.receive(&session).unwrap().unwrap();
    assert!(matches!(pending.action, Action::Stop));
    assert!(
        pending
            .reply(
                &session,
                Ok(serde_json::json!({"stop_accepted": true, "unread": "x".repeat(1_000_000)})),
            )
            .is_err()
    );
    assert!(server.receive(&session).unwrap().is_none());

    let mut caller = UnixStream::connect(control_socket(&path)).unwrap();
    caller.write_all(b"{\"action\":").unwrap();
    drop(caller);
    assert!(server.receive(&session).unwrap().is_none());
}

#[test]
fn control_server_never_replaces_or_removes_foreign_sockets_or_links() {
    let (_temp, registry, workspace, session) = fixture();
    let path = registry.runtime_path(&workspace);
    private::directory(&path).unwrap();
    parcela::sockets::prepare(&control_socket(&path)).unwrap();
    let foreign = UnixListener::bind(control_socket(&path)).unwrap();
    assert!(Server::bind(&path).is_err());
    assert!(control_socket(&path).exists());
    drop(foreign);
    fs::remove_file(control_socket(&path)).unwrap();
    let owner = Server::bind(&path).unwrap();
    fs::rename(control_socket(&path), path.join("original.sock")).unwrap();
    let replacement = UnixListener::bind(control_socket(&path)).unwrap();
    drop(owner);
    assert!(control_socket(&path).exists());
    drop(replacement);
    fs::remove_file(control_socket(&path)).unwrap();
    symlink(path.join("original.sock"), control_socket(&path)).unwrap();
    assert!(control::request(&path, &session, Action::Stop).is_err());
    assert!(control_socket(&path).is_symlink());
    fs::remove_file(control_socket(&path)).unwrap();
}

#[test]
fn backend_output_is_bounded_and_control_commands_have_deadlines() {
    let output = process::capture(
        Command::new("sh").args(["-c", "printf output; printf error >&2"]),
        Duration::from_secs(1),
        1024,
    )
    .unwrap();
    assert_eq!(output.stdout, b"output");
    assert_eq!(output.stderr, b"error");
    assert!(
        process::capture(
            Command::new("sh").args(["-c", "printf oversized"]),
            Duration::from_secs(1),
            2
        )
        .is_err()
    );
    assert!(
        process::capture(
            Command::new("sh").args(["-c", "sleep 10"]),
            Duration::from_millis(30),
            1024
        )
        .is_err()
    );
}

#[test]
fn malformed_owner_health_response_stays_unknown() {
    let (_temp, registry, workspace, session) = fixture();
    let server = Server::bind(&registry.runtime_path(&workspace)).unwrap();
    let owner = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let pending = loop {
            if let Some(pending) = server.receive(&session).unwrap() {
                break pending;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "malformed-response fixture received no request"
            );
            thread::sleep(Duration::from_millis(10));
        };
        pending
            .reply(&session, Ok(serde_json::json!({"state": "ready"})))
            .unwrap();
    });
    let status = lifecycle::status(&registry, &workspace).unwrap();
    owner.join().unwrap();
    assert_eq!(status["state"], "unknown");
    assert_eq!(status["live_health_verified"], false);
}

#[test]
fn an_interrupted_stop_finishes_by_stopping_recorded_survivors() {
    use parcela::owner::{Phase, Record};

    let (_temp, mut registry, workspace, session) = fixture();
    let path = registry.runtime_path(&workspace);
    private::directory(&path).unwrap();
    let lock = private::Lock::acquire(&path.join("owner.lock")).unwrap();
    let control = Server::bind(&path).unwrap();
    let mut child = Command::new("sleep").arg("60").spawn().unwrap();
    let identity = process::Identity::read(child.id()).unwrap().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    let mut record = Record {
        session: session.clone(),
        phase: Phase::Running,
        owner: identity.clone(),
        lock: lock.identity().unwrap(),
        control: private::Identity::read(&control_socket(&path)).unwrap(),
        children: web(&identity),
        processes: vec![identity],
        manifest: Manifest::parse(
            "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
        )
        .unwrap(),
        diagnostic: None,
        share: Default::default(),
    };
    record.save(&path).unwrap();
    drop(control);
    drop(lock);

    let mut survivor = process::OwnedChild::spawn(Command::new("sleep").arg("60")).unwrap();
    let survivor_identity = process::Identity::read(survivor.id()).unwrap().unwrap();
    record.processes.push(survivor_identity.clone());
    record.phase = Phase::Stopping;
    record.save(&path).unwrap();
    registry
        .session_status(&session, SessionStatus::Stopping)
        .unwrap();
    // The survivor is signaled through its recorded identity, never through a bare pid.
    assert_eq!(
        lifecycle::down(&registry, &workspace).unwrap()["state"],
        "stopped"
    );
    assert!(!survivor_identity.alive());
    survivor.stop(libc::SIGKILL).unwrap();
    assert_eq!(
        registry.session(&workspace.id).unwrap().unwrap().status,
        SessionStatus::Stopped
    );
    assert_eq!(Record::read(&path, &session).unwrap().phase, Phase::Stopped);
    assert!(!control_socket(&path).exists());
    assert_eq!(
        lifecycle::down(&registry, &workspace).unwrap()["already_stopped"],
        true
    );
}

#[test]
fn contending_stop_waits_for_same_generation_completion() {
    contending_stop(false);
}

#[test]
fn contending_stop_refuses_a_successor_generation() {
    contending_stop(true);
}

fn contending_stop(replace_session: bool) {
    use parcela::owner::{Phase, Record};

    let (_temp, mut registry, workspace, session) = fixture();
    let path = registry.runtime_path(&workspace);
    private::directory(&path).unwrap();
    let lock = private::Lock::acquire(&path.join("owner.lock")).unwrap();
    let control = Server::bind(&path).unwrap();
    let mut child = process::OwnedChild::spawn(Command::new("sleep").arg("60")).unwrap();
    let identity = process::Identity::read(child.id()).unwrap().unwrap();
    child.stop(libc::SIGTERM).unwrap();
    let mut record = Record {
        session: session.clone(),
        phase: Phase::Stopping,
        owner: identity.clone(),
        lock: lock.identity().unwrap(),
        control: private::Identity::read(&control_socket(&path)).unwrap(),
        children: web(&identity),
        processes: vec![identity],
        manifest: Manifest::parse(
            "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
        )
        .unwrap(),
        diagnostic: None,
        share: Default::default(),
    };
    record.save(&path).unwrap();
    drop(control);
    registry
        .session_status(&session, SessionStatus::Stopping)
        .unwrap();
    let state = registry.state_path().to_path_buf();
    let expected = session.clone();
    let target = workspace.clone();
    let owner = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        let mut writer = Registry::open(&state, true).unwrap().unwrap();
        record.phase = Phase::Stopped;
        record.save(&path).unwrap();
        writer
            .session_status(&expected, SessionStatus::Stopped)
            .unwrap();
        let successor = replace_session.then(|| {
            writer
                .begin_session(
                    &target,
                    expected.router_token.as_deref().unwrap(),
                    32002,
                    "up",
                    expected.public_url.as_deref().unwrap(),
                )
                .unwrap()
        });
        drop(lock);
        successor
    });
    let result = lifecycle::down(&registry, &workspace);
    let successor = owner.join().unwrap();
    let current = registry.session(&workspace.id).unwrap().unwrap();
    if let Some(successor) = successor {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("generation changed")
        );
        assert_eq!(current.token, successor.token);
        assert_eq!(current.status, SessionStatus::Starting);
    } else {
        assert_eq!(result.unwrap()["state"], "stopped");
        assert_eq!(current.token, session.token);
    }
}

#[test]
fn waiting_stop_reaches_the_recovered_same_generation_controller() {
    waiting_stop(false);
}

#[test]
fn waiting_stop_never_reaches_a_replaced_control_socket() {
    waiting_stop(true);
}

fn waiting_stop(replace_control: bool) {
    use parcela::owner::{Phase, Record};
    use std::time::Instant;

    let (_temp, mut registry, workspace, session) = fixture();
    let path = registry.runtime_path(&workspace);
    private::directory(&path).unwrap();
    let lock = private::Lock::acquire(&path.join("owner.lock")).unwrap();
    let control = Server::bind(&path).unwrap();
    let mut child = process::OwnedChild::spawn(Command::new("sleep").arg("60")).unwrap();
    let identity = process::Identity::read(child.id()).unwrap().unwrap();
    child.stop(libc::SIGTERM).unwrap();
    let mut record = Record {
        session: session.clone(),
        phase: Phase::Starting,
        owner: process::Identity::read(std::process::id()).unwrap().unwrap(),
        lock: lock.identity().unwrap(),
        control: private::Identity::read(&control_socket(&path)).unwrap(),
        children: web(&identity),
        processes: vec![identity],
        manifest: Manifest::parse(
            "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
        )
        .unwrap(),
        diagnostic: None,
        share: Default::default(),
    };
    record.save(&path).unwrap();
    let (control, retained_control) = if replace_control {
        fs::rename(control_socket(&path), path.join("retained.sock")).unwrap();
        (Server::bind(&path).unwrap(), Some(control))
    } else {
        (control, None)
    };
    let retained_path = path.join("retained.sock");
    let state = registry.state_path().to_path_buf();
    let expected = session.clone();
    let owner = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let admitted = loop {
            if let Some(pending) = control.receive(&expected).unwrap() {
                assert!(matches!(pending.action, Action::Stop));
                record.phase = Phase::Stopping;
                record.save(&path).unwrap();
                let mut writer = Registry::open(&state, true).unwrap().unwrap();
                writer
                    .session_status(&expected, SessionStatus::Stopping)
                    .unwrap();
                pending
                    .reply(&expected, Ok(serde_json::json!({"stop_accepted":true})))
                    .unwrap();
                record.phase = Phase::Stopped;
                record.save(&path).unwrap();
                writer
                    .session_status(&expected, SessionStatus::Stopped)
                    .unwrap();
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(5));
        };
        drop(control);
        drop(lock);
        admitted
    });
    let result = parcela::runtime::recover(&mut registry, &workspace, &session, true);
    let admitted = owner.join().unwrap();
    if replace_control {
        assert!(!admitted, "stop reached a replaced control socket");
        assert!(result.is_err());
        assert_eq!(
            registry.session(&workspace.id).unwrap().unwrap().status,
            SessionStatus::Starting
        );
        assert!(retained_path.exists());
    } else {
        assert!(admitted, "stop never reached the recovered controller");
        assert_eq!(result.unwrap()["state"], "stopped");
    }
    drop(retained_control);
    assert_eq!(
        registry.session(&workspace.id).unwrap().unwrap().token,
        session.token
    );
}

#[test]
fn fragmented_owner_request_waits_for_the_complete_frame() {
    use std::{
        io::{Read, Write},
        os::unix::net::UnixStream,
    };

    let (_temp, registry, workspace, session) = fixture();
    let path = registry.runtime_path(&workspace);
    let server = Server::bind(&path).unwrap();
    let mut caller = UnixStream::connect(control_socket(&path)).unwrap();
    caller
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut frame = serde_json::to_vec(&serde_json::json!({"environment_id": session.environment_id, "token": session.token, "generation": session.generation, "action": {"type": "status"}})).unwrap();
    frame.push(b'\n');
    let split = frame.len() / 2;
    caller.write_all(&frame[..split]).unwrap();
    let writer = thread::spawn(move || -> anyhow::Result<serde_json::Value> {
        thread::sleep(Duration::from_millis(40));
        caller.write_all(&frame[split..])?;
        let mut response = String::new();
        caller.read_to_string(&mut response)?;
        Ok(serde_json::from_str(&response)?)
    });
    let received = server.receive(&session).unwrap();
    if let Some(pending) = received {
        assert!(matches!(pending.action, Action::Status));
        pending
            .reply(&session, Ok(serde_json::json!({"accepted": true})))
            .unwrap();
    } else {
        let _ = writer.join();
        panic!("complete owner request arriving within the read budget must be admitted");
    }
    let response = writer.join().unwrap().unwrap();
    assert_eq!(response["result"], serde_json::json!({"accepted": true}));
    assert_eq!(response["error"], serde_json::Value::Null);
}

#[test]
fn unfinished_open_owner_request_is_rejected_within_a_bounded_wait() {
    use std::{
        io::{Read, Write},
        os::unix::net::UnixStream,
    };

    let (_temp, registry, workspace, session) = fixture();
    let path = registry.runtime_path(&workspace);
    let server = Server::bind(&path).unwrap();
    let mut caller = UnixStream::connect(control_socket(&path)).unwrap();
    caller
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    caller.write_all(b"{\"action\":").unwrap();
    let started = std::time::Instant::now();
    assert!(server.receive(&session).unwrap().is_none());
    assert!(started.elapsed() < Duration::from_secs(1));
    let mut response = String::new();
    caller.read_to_string(&mut response).unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(response["error"].is_string());
    assert_eq!(response["result"], serde_json::Value::Null);
}

#[test]
fn rebooted_runs_converge_after_device_renumbering_but_not_replaced_locks() {
    use parcela::owner::{Phase, Record};
    use serde_json::json;

    for case in [
        "down",
        "up",
        "same-boot",
        "replaced-lock",
        "live-process",
        "live-child",
        "replaced-socket",
    ] {
        let (_temp, mut registry, workspace, session) = fixture();
        registry
            .session_status(&session, SessionStatus::Ready)
            .unwrap();
        let session = registry.session(&workspace.id).unwrap().unwrap();
        let path = registry.runtime_path(&workspace);
        private::directory(&path).unwrap();
        let lock = private::Lock::acquire(&path.join("owner.lock")).unwrap();
        let control = Server::bind(&path).unwrap();
        let socket = control_socket(&path);
        let control_identity = private::Identity::read(&socket).unwrap();
        drop(control);
        // Reboot removes the temporary control socket, but leaves the persistent lock file.
        if socket.exists() {
            fs::remove_file(&socket).unwrap();
        }
        let replacement = (case == "replaced-socket").then(|| UnixListener::bind(&socket).unwrap());
        let current = process::Identity::read(std::process::id())
            .unwrap()
            .unwrap();
        let mut old = serde_json::to_value(&current).unwrap();
        old["boot"] = json!("previous-boot");
        let old: process::Identity = serde_json::from_value(old).unwrap();
        let mut dead = Command::new("sleep").arg("60").spawn().unwrap();
        let dead_identity = process::Identity::read(dead.id()).unwrap().unwrap();
        dead.kill().unwrap();
        dead.wait().unwrap();
        let mut recorded_lock = serde_json::to_value(lock.identity().unwrap()).unwrap();
        recorded_lock["device"] = json!(recorded_lock["device"].as_u64().unwrap() + 1);
        if case == "replaced-lock" {
            recorded_lock["inode"] = json!(recorded_lock["inode"].as_u64().unwrap() + 1);
        }
        let record = Record {
            session: session.clone(),
            phase: Phase::Running,
            owner: if case == "same-boot" {
                dead_identity
            } else {
                old.clone()
            },
            lock: serde_json::from_value(recorded_lock).unwrap(),
            control: control_identity,
            children: web(if case == "live-child" { &current } else { &old }),
            processes: vec![if case == "live-process" { current.clone() } else { old }],
            manifest: Manifest::parse("version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n").unwrap(),
            diagnostic: None,
            share: Default::default(),
        };
        record.save(&path).unwrap();
        drop(lock);
        let result = if case == "up" {
            let lock = private::Lock::acquire(&path.join("owner.lock")).unwrap();
            parcela::runtime::converge_crashed_session(&mut registry, &workspace, &lock).map(
                |converged| {
                    assert!(converged);
                },
            )
        } else {
            lifecycle::down(&registry, &workspace).map(|_| ())
        };
        let succeeds = matches!(case, "down" | "up");
        assert_eq!(result.is_ok(), succeeds, "{case}: {result:?}");
        assert_eq!(
            registry.session(&workspace.id).unwrap().unwrap().status,
            if succeeds {
                SessionStatus::Stopped
            } else {
                SessionStatus::Ready
            }
        );
        assert!(current.alive(), "reused PIDs must never be signaled");
        if succeeds {
            assert_eq!(
                lifecycle::down(&registry, &workspace).unwrap()["already_stopped"],
                true
            );
        }
        if replacement.is_some() {
            assert!(socket.exists(), "replacement socket must be preserved");
        }
    }
}

#[test]
fn down_stops_what_survives_a_crashed_owner_by_recorded_identity() {
    use parcela::owner::{Phase, Record};

    let (_temp, mut registry, workspace, session) = fixture();
    registry
        .session_status(&session, SessionStatus::Ready)
        .unwrap();
    let path = registry.runtime_path(&workspace);
    private::directory(&path).unwrap();
    let lock = private::Lock::acquire(&path.join("owner.lock")).unwrap();
    let sockets = parcela::sockets::environment(&path).unwrap();
    parcela::sockets::prepare(&sockets.control).unwrap();
    let control = UnixListener::bind(&sockets.control).unwrap();

    let mut owner = Command::new("sleep").arg("60").spawn().unwrap();
    let owner_identity = process::Identity::read(owner.id()).unwrap().unwrap();
    let mut supervisor = Command::new("sh")
        .args(["-c", "sleep 60 & sleep 60 & wait"])
        .spawn()
        .unwrap();
    let supervisor_identity = process::Identity::read(supervisor.id()).unwrap().unwrap();
    let tree = loop {
        let tree = process::tree(std::slice::from_ref(&supervisor_identity)).unwrap();
        if tree.len() == 3 {
            break tree;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut record = Record {
        session: registry.session(&workspace.id).unwrap().unwrap(),
        phase: Phase::Running,
        owner: owner_identity.clone(),
        lock: lock.identity().unwrap(),
        control: private::Identity::read(&sockets.control).unwrap(),
        children: web(&supervisor_identity),
        processes: tree.clone(),
        manifest: Manifest::parse(
            "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
        )
        .unwrap(),
        diagnostic: None,
        share: Default::default(),
    };
    record.save(&path).unwrap();
    drop(control);
    drop(lock);
    owner.kill().unwrap();
    owner.wait().unwrap();

    let survivor = tree
        .iter()
        .find(|identity| identity.pid != supervisor_identity.pid)
        .unwrap()
        .clone();
    for identity in &tree {
        if identity != &survivor {
            unsafe { libc::kill(identity.pid as i32, libc::SIGKILL) };
        }
    }
    supervisor.wait().unwrap();
    assert!(!supervisor_identity.alive() && survivor.alive());
    assert!(
        registry
            .begin_session(
                &workspace,
                &Uuid::new_v4().to_string(),
                32001,
                "up",
                "https://auth-app.localhost",
            )
            .is_err(),
        "a crashed environment blocks up until it converges"
    );
    // The survivor outlived its parent, so it is stopped by its recorded identity.
    let stopped = lifecycle::down(&registry, &workspace).unwrap();
    assert_eq!(stopped["state"], "stopped");
    assert!(!survivor.alive());
    assert_eq!(stopped["generation"], session.generation);
    assert_eq!(
        registry.session(&workspace.id).unwrap().unwrap().status,
        SessionStatus::Stopped
    );
    record = Record::read(&path, &record.session).unwrap();
    assert_eq!(record.phase, Phase::Stopped);
    assert!(!sockets.control.exists());
    assert_eq!(
        lifecycle::down(&registry, &workspace).unwrap()["already_stopped"],
        true
    );
    let next = registry
        .begin_session(
            &workspace,
            &Uuid::new_v4().to_string(),
            32001,
            "up",
            "https://auth-app.localhost",
        )
        .unwrap();
    assert_eq!(next.generation, session.generation + 1);
}
