//! Backend port leases: a free loopback port per service endpoint and named port, held open
//! until the backend starts.
use crate::{
    config::Manifest,
    registry::{Endpoint, Registry, Session, SessionStatus, Workspace},
    router::Router,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener},
    os::fd::{FromRawFd, OwnedFd},
};
use uuid::Uuid;

pub(crate) type Allocation = (Vec<Endpoint>, BTreeMap<String, u16>, Vec<TcpListener>);

/// Claim a free backend port for every service endpoint and named port, skipping every port
/// another active session leases. Named ports prefer the environment's previous in-range port.
pub(crate) fn allocate(
    registry: &Registry,
    manifest: &Manifest,
    workspace: &Workspace,
    router: Option<&Router>,
    port: Option<u16>,
) -> Result<Allocation> {
    let unavailable: BTreeSet<_> = registry
        .sessions()?
        .iter()
        .filter(|session| session.status != SessionStatus::Stopped)
        .flat_map(Session::leased_ports)
        .chain(router.map(|router| router.front_door.https))
        .collect();
    let mut endpoints = Vec::new();
    let mut probes = Vec::new();
    for (service, hostname) in manifest.service_hosts(&workspace.hostname)? {
        let probe =
            match port.filter(|_| Some(service.as_str()) == manifest.default_service.as_deref()) {
                Some(requested) => claim(requested).with_context(|| {
                    format!(
                        "--backend-port {requested} is already in use by {}; pick another",
                        crate::process::port_holder(requested)
                    )
                })?,
                None => free_port(&unavailable)?,
            };
        let backend_port = probe.local_addr()?.port();
        ensure!(
            backend_port >= 1024 && router.is_none_or(|router| !router.listens_on(backend_port)),
            "backend endpoint conflicts with privileged/router ports"
        );
        endpoints.push(Endpoint {
            service,
            public_url: router
                .context("HTTP service needs a router")?
                .url(&hostname),
            hostname,
            backend_port,
        });
        probes.push(probe);
    }
    let previous = registry
        .session(&workspace.id)?
        .map(|session| session.ports)
        .unwrap_or_default();
    let mut ports = BTreeMap::new();
    for name in &manifest.ports {
        let reused = previous
            .get(name)
            .filter(|port| BACKEND_PORTS.contains(port) && !unavailable.contains(port))
            .and_then(|port| claim(*port));
        let probe = match reused {
            Some(probe) => probe,
            None => free_port(&unavailable)?,
        };
        ports.insert(name.clone(), probe.local_addr()?.port());
        probes.push(probe);
    }
    Ok((endpoints, ports, probes))
}

/// Below every OS ephemeral range (Linux 32768+, macOS and Windows 49152+), so a process
/// binding port 0 cannot take an allocated port before the backend binds it. It holds no
/// WHATWG bad ports and stays clear of rb-portless's 4000-4999 range.
const BACKEND_PORTS: std::ops::RangeInclusive<u16> = 20000..=29999;

fn free_port(unavailable: &BTreeSet<u16>) -> Result<TcpListener> {
    let start =
        BACKEND_PORTS.start() + (Uuid::new_v4().as_u128() % BACKEND_PORTS.len() as u128) as u16;
    candidates(start, unavailable)
        .find_map(claim)
        .with_context(|| {
            format!(
                "no free backend port in {}-{}; foreign listeners are never stopped",
                BACKEND_PORTS.start(),
                BACKEND_PORTS.end()
            )
        })
}

/// Every allowed backend port once, from `start` upward and wrapping, so concurrent
/// allocations rarely contend for the same port.
fn candidates(start: u16, unavailable: &BTreeSet<u16>) -> impl Iterator<Item = u16> {
    (start..=*BACKEND_PORTS.end())
        .chain(*BACKEND_PORTS.start()..start)
        .filter(|port| !unavailable.contains(port))
}

/// Hold `port` on 127.0.0.1 unless any socket owns it on a loopback or wildcard address.
/// std binds with SO_REUSEADDR, which lets 127.0.0.1 succeed beside a 0.0.0.0, [::] or [::1] owner.
pub(crate) fn claim(port: u16) -> Option<TcpListener> {
    let addresses: [IpAddr; 3] = [
        Ipv4Addr::UNSPECIFIED.into(),
        Ipv6Addr::UNSPECIFIED.into(),
        Ipv6Addr::LOCALHOST.into(),
    ];
    let occupied = addresses.into_iter().any(|ip| {
        bind_only(SocketAddr::new(ip, port))
            .is_err_and(|error| error.kind() == std::io::ErrorKind::AddrInUse)
    });
    if occupied {
        return None;
    }
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok()
}

/// Bind with SO_REUSEADDR like std, but never listen, so a wildcard probe accepts nothing.
fn bind_only(address: SocketAddr) -> std::io::Result<OwnedFd> {
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let length = match address {
        SocketAddr::V4(address) => {
            let raw = unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in) };
            raw.sin_family = libc::AF_INET as _;
            raw.sin_port = address.port().to_be();
            raw.sin_addr.s_addr = u32::from(*address.ip()).to_be();
            std::mem::size_of::<libc::sockaddr_in>()
        }
        SocketAddr::V6(address) => {
            let raw = unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in6) };
            raw.sin6_family = libc::AF_INET6 as _;
            raw.sin6_port = address.port().to_be();
            raw.sin6_addr.s6_addr = address.ip().octets();
            std::mem::size_of::<libc::sockaddr_in6>()
        }
    };
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    {
        storage.ss_len = length as u8;
    }
    let fd = unsafe { libc::socket(storage.ss_family as _, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let enable: libc::c_int = 1;
    let bound = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            (&enable as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        ) == 0
            && libc::bind(
                fd,
                (&storage as *const libc::sockaddr_storage).cast(),
                length as libc::socklen_t,
            ) == 0
    };
    if bound {
        Ok(socket)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IPV6_LOOPBACK_PORT: u16 = 47343;
    const IPV4_LOOPBACK_PORT: u16 = 47344;

    #[test]
    fn candidates_cover_the_range_once_without_leased_ports() {
        let leased = BTreeSet::from([20000, 25000, 29999]);
        let ports: Vec<_> = candidates(29998, &leased).collect();
        assert_eq!(&ports[..3], &[29998, 20001, 20002]);
        assert_eq!(ports.len(), BACKEND_PORTS.len() - leased.len());
        assert_eq!(ports.iter().collect::<BTreeSet<_>>().len(), ports.len());
        assert!(
            ports
                .iter()
                .all(|port| BACKEND_PORTS.contains(port) && !leased.contains(port))
        );
    }

    #[test]
    fn free_port_is_outside_the_ephemeral_ranges_and_skips_leases() {
        let leased: BTreeSet<_> = BACKEND_PORTS.filter(|port| port % 2 == 0).collect();
        let probe = free_port(&leased).unwrap();
        let port = probe.local_addr().unwrap().port();
        assert!(BACKEND_PORTS.contains(&port) && port % 2 == 1, "{port}");
        assert!(probe.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn exhausted_range_is_a_clear_error() {
        let error = free_port(&BACKEND_PORTS.collect()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "no free backend port in 20000-29999; foreign listeners are never stopped"
        );
    }

    #[test]
    fn a_port_listened_on_either_loopback_family_is_busy() {
        let _v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, IPV6_LOOPBACK_PORT)).unwrap();
        assert!(claim(IPV6_LOOPBACK_PORT).is_none(), "[::1] listener");
        let _v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, IPV4_LOOPBACK_PORT)).unwrap();
        assert!(claim(IPV4_LOOPBACK_PORT).is_none(), "127.0.0.1 listener");
    }

    /// Bound without listening, so the wildcard owners accept nothing from the network.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_port_owned_on_a_wildcard_address_is_busy() {
        let _v4 = bind_only((Ipv4Addr::UNSPECIFIED, 47341).into()).unwrap();
        assert!(claim(47341).is_none(), "0.0.0.0 owner");
        let _v6 = bind_only((Ipv6Addr::UNSPECIFIED, 47342).into()).unwrap();
        assert!(claim(47342).is_none(), "[::] owner");
    }
}
