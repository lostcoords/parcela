//! Parcela's Rust control plane. Caddy is the proxy; `environment::supervisor` runs each
//! environment's processes.
//!
//! - `project`: what a checkout declares, and its VCS checkouts and data.
//! - `state`: durable machine state: the registry, port leases, host settings, private files.
//! - `environment`: one environment's run: its owner, process graph, control socket and logs.
//! - `proxy`: the shared proxy: Caddy routes, the daemon, the dashboard and the badge injector.
//! - `share`: LAN, Tailscale and domain sharing for one run.
pub mod environment;
#[cfg(test)]
mod fuzz;
pub mod http;
pub mod paint;
pub mod process;
pub mod project;
pub mod proxy;
pub mod share;
pub mod state;
