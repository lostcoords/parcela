use parcela::{
    http,
    process::{Listeners, listener_identity},
};
use std::{
    io::{Read, Write},
    net::{Ipv6Addr, TcpListener},
    process::Command,
    sync::{Mutex, MutexGuard},
    thread,
};

/// macOS marks a new socket close-on-exec in a second step, so a child spawned by one test in that
/// gap inherits another test's listener and shows up as its owner. These tests take turns.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const IPV6_ONLY_PORT: u16 = 47331;
const DUAL_STACK_PORT: u16 = 47332;
const FOREIGN_PORT: u16 = 47333;
const READINESS_PORT: u16 = 47334;

#[test]
fn ipv6_loopback_listener_owned_by_the_environment_is_accepted() {
    let _serial = serial();
    let _listener = TcpListener::bind((Ipv6Addr::LOCALHOST, IPV6_ONLY_PORT)).unwrap();
    assert_eq!(
        listener_identity(IPV6_ONLY_PORT, &[std::process::id()]).unwrap(),
        vec![std::process::id()]
    );
}

#[test]
fn localhost_listener_on_both_loopback_families_is_accepted() {
    let _serial = serial();
    let _v4 = TcpListener::bind(("127.0.0.1", DUAL_STACK_PORT)).unwrap();
    let _v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, DUAL_STACK_PORT)).unwrap();
    assert_eq!(
        listener_identity(DUAL_STACK_PORT, &[std::process::id()]).unwrap(),
        vec![std::process::id()]
    );
}

#[test]
fn loopback_listener_outside_the_environment_tree_is_foreign() {
    let _serial = serial();
    let _listener = TcpListener::bind((Ipv6Addr::LOCALHOST, FOREIGN_PORT)).unwrap();
    let mut sibling = Command::new("sleep").arg("30").spawn().unwrap();
    let error = listener_identity(FOREIGN_PORT, &[sibling.id()]).unwrap_err();
    sibling.kill().unwrap();
    sibling.wait().unwrap();
    assert_eq!(
        error.to_string(),
        "allocated endpoint was acquired by a foreign listener; refusing route registration"
    );
}

#[test]
fn readiness_reaches_an_ipv6_only_backend() {
    let _serial = serial();
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, READINESS_PORT)).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        assert!(request.starts_with(b"GET /up HTTP/1.1\r\nHost: demo.localhost\r\n"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
    });
    let address = Listeners::inspect([READINESS_PORT])
        .unwrap()
        .address(READINESS_PORT)
        .unwrap();
    assert!(http::readiness(address, "demo.localhost", "/up").unwrap());
    server.join().unwrap();
}
