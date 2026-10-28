use clap::builder::styling::{AnsiColor, Styles};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Help colors: amber headings like the docs site, cyan commands and flags.
const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().bold())
    .usage(AnsiColor::Yellow.on_default().bold())
    .literal(AnsiColor::Cyan.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default())
    .error(AnsiColor::Red.on_default().bold())
    .valid(AnsiColor::Green.on_default())
    .invalid(AnsiColor::Yellow.on_default());

/// Deployable dev environments for your AI agents
#[derive(Parser)]
#[command(version = env!("PARCELA_VERSION"), styles = STYLES)]
pub struct Args {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(clap::Args, Clone)]
#[command(next_help_heading = "Global options")]
pub struct Global {
    /// Checkout directory
    #[arg(short = 'C', long, global = true, default_value = ".")]
    pub path: PathBuf,
    /// State directory [env: PARCELA_STATE_DIR]
    #[arg(long, global = true)]
    pub state_dir: Option<PathBuf>,
    /// Environment by name, URL or ID, instead of --path
    #[arg(short, long, global = true, conflicts_with = "path")]
    pub workspace: Option<String>,
    /// Print JSON (default when not a terminal)
    #[arg(long, global = true)]
    pub json: bool,
}

/// A parsed invocation with its command resolved.
pub struct Cli {
    pub path: PathBuf,
    pub state_dir: Option<PathBuf>,
    pub workspace: Option<String>,
    pub json: bool,
    pub command: Command,
}

impl Cli {
    pub fn new(global: Global, command: Command) -> Self {
        Self {
            path: global.path,
            state_dir: global.state_dir,
            workspace: global.workspace,
            json: global.json,
            command,
        }
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a parcela.yaml
    Init {
        /// Project name
        #[arg(long)]
        name: Option<String>,
        /// Web service command
        #[arg(long)]
        command: Option<String>,
    },
    /// Create and register a new Git worktree or jj workspace without starting it
    Create {
        /// Preview environment name
        name: String,
        /// New checkout directory (default: a sibling named <repository>-<name>)
        #[arg(long)]
        directory: Option<PathBuf>,
        /// Source revision (default: Git HEAD or jj @); Git worktrees are detached
        #[arg(long)]
        revision: Option<String>,
        /// Restore a named data snapshot before the first start
        #[arg(long)]
        from: Option<String>,
    },
    /// Save declared data from a stopped environment to an immutable named snapshot
    Snapshot {
        /// New immutable snapshot name
        name: String,
    },
    /// Restore a data snapshot into a fresh registered environment; never replaces data
    Restore {
        /// Snapshot to restore
        name: String,
    },
    /// Start the environment
    Up {
        /// Environment name (default: the worktree name)
        #[arg(long, conflicts_with_all = ["primary", "workspace"])]
        name: Option<String>,
        /// Use the project's main URL
        #[arg(long, conflicts_with = "workspace")]
        primary: bool,
        /// Run in the background
        #[arg(short, long)]
        detach: bool,
        /// Run the setup hook again
        #[arg(long)]
        setup: bool,
        /// Port for the main service
        #[arg(long, value_parser = clap::value_parser!(u16).range(1024..))]
        backend_port: Option<u16>,
        /// Also serve at .local names on the LAN
        #[arg(long, env = "PARCELA_LAN", value_parser = clap::builder::FalseyValueParser::new())]
        lan: bool,
        /// LAN address for --lan (default: detected)
        #[arg(long, value_name = "IP")]
        lan_ip: Option<std::net::Ipv4Addr>,
        /// Also serve on your tailnet with tailscale serve
        #[arg(long, env = "PARCELA_TAILSCALE", value_parser = clap::builder::FalseyValueParser::new())]
        tailscale: bool,
        /// Also serve at <name>.<DOMAIN> through your own HTTPS proxy, which forwards to --domain-port
        #[arg(long, env = "PARCELA_DOMAIN", value_name = "DOMAIN")]
        domain: Option<String>,
        /// Loopback HTTP port for --domain [default: 1480]
        #[arg(long, env = "PARCELA_DOMAIN_PORT", requires = "domain", value_parser = clap::value_parser!(u16).range(1024..))]
        domain_port: Option<u16>,
        /// Readiness timeout in seconds, instead of readiness_timeout_seconds
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86400), hide = true)]
        readiness_timeout: Option<u64>,
        #[arg(long, hide = true)]
        launch_file: Option<PathBuf>,
    },
    /// Stop the environment
    #[command(visible_alias = "stop")]
    Down {
        /// Stop all of this project's environments
        #[arg(long, conflicts_with = "workspace")]
        all: bool,
        /// Stop all environments
        #[arg(long, conflicts_with_all = ["workspace", "all"])]
        all_projects: bool,
    },
    /// Restart the environment
    Restart {
        /// Run in the background
        #[arg(short, long)]
        detach: bool,
        /// Run the setup hook again
        #[arg(long)]
        setup: bool,
        /// Port for the main service
        #[arg(long, value_parser = clap::value_parser!(u16).range(1024..))]
        backend_port: Option<u16>,
        /// Also serve at .local names on the LAN
        #[arg(long, env = "PARCELA_LAN", value_parser = clap::builder::FalseyValueParser::new())]
        lan: bool,
        /// LAN address for --lan (default: detected)
        #[arg(long, value_name = "IP")]
        lan_ip: Option<std::net::Ipv4Addr>,
        /// Also serve on your tailnet with tailscale serve
        #[arg(long, env = "PARCELA_TAILSCALE", value_parser = clap::builder::FalseyValueParser::new())]
        tailscale: bool,
        /// Also serve at <name>.<DOMAIN> through your own HTTPS proxy, which forwards to --domain-port
        #[arg(long, env = "PARCELA_DOMAIN", value_name = "DOMAIN")]
        domain: Option<String>,
        /// Loopback HTTP port for --domain [default: 1480]
        #[arg(long, env = "PARCELA_DOMAIN_PORT", requires = "domain", value_parser = clap::value_parser!(u16).range(1024..))]
        domain_port: Option<u16>,
        /// Readiness timeout in seconds, instead of readiness_timeout_seconds
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86400), hide = true)]
        readiness_timeout: Option<u64>,
        #[arg(long, hide = true)]
        launch_file: Option<PathBuf>,
    },
    /// Show the environment's status
    Status,
    /// List environments
    Ls,
    /// Show logs
    Logs {
        /// Service name
        service: Option<String>,
        /// Lines to show per service
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=10000))]
        tail: u32,
        /// Follow log output
        #[arg(short, long)]
        follow: bool,
    },
    /// Print the environment's URL
    Url {
        /// Service name
        service: Option<String>,
    },
    /// Open the environment's URL in the browser
    Open {
        /// Service name
        service: Option<String>,
    },
    /// Remove a stopped environment
    Rm {
        /// Remove environments whose checkout is gone
        #[arg(long, conflicts_with = "workspace")]
        prune: bool,
    },
    /// Check the setup
    Doctor {
        /// Caddy binary to probe instead of caddy on PATH.
        #[arg(long, hide = true)]
        caddy: Option<String>,
    },
    /// Manage the HTTPS proxy
    Proxy {
        #[command(subcommand)]
        command: ProxyCommand,
    },
    /// Trust the local certificate authority
    Trust {
        /// Print the CA certificate path
        #[arg(long)]
        print: bool,
        /// Caddy binary.
        #[arg(long, default_value = "caddy", hide = true)]
        caddy: String,
    },
    /// Run a command as the web service
    Run {
        /// Environment name (default: the worktree name)
        #[arg(long, conflicts_with_all = ["primary", "workspace"])]
        name: Option<String>,
        /// Use the project's main URL
        #[arg(long, conflicts_with = "workspace")]
        primary: bool,
        /// Port for the command
        #[arg(long, value_parser = clap::value_parser!(u16).range(1024..))]
        backend_port: Option<u16>,
        /// Readiness timeout in seconds
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400), hide = true)]
        readiness_timeout: u64,
        /// Command and arguments
        #[arg(last = true, required = true, allow_hyphen_values = true)]
        argv: Vec<String>,
    },
    /// Run a task
    Task {
        /// Task name
        name: String,
        /// Task arguments
        #[arg(last = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Upgrade Parcela
    Upgrade {
        /// Install the newest build of main
        #[arg(long)]
        nightly: bool,
    },
    /// Reattach to services left running
    #[command(hide = true)]
    Recover {
        /// Run in the background
        #[arg(short, long)]
        detach: bool,
    },
    /// Print the environment's details
    #[command(hide = true)]
    Context,
    /// Print the agent skill
    #[command(long_flag = "skill")]
    Skill,
    /// Print shell completions
    Completion {
        /// Shell
        shell: clap_complete::Shell,
    },
    /// Serve a Compose service's container port on $PORT
    #[command(hide = true)]
    ComposeForward { service: String, port: u16 },
    /// Manage anonymous usage data
    Telemetry {
        #[command(subcommand)]
        command: Option<TelemetryCommand>,
    },
}

#[derive(Subcommand, Clone, Copy)]
pub enum TelemetryCommand {
    /// Show whether it is on
    Status,
    /// Turn it on
    Enable,
    /// Turn it off
    Disable,
}

#[derive(Subcommand)]
pub enum ProxyCommand {
    /// Start the proxy
    Start {
        #[command(flatten)]
        listen: Listen,
    },
    /// Stop the proxy
    Stop,
    /// Restart the proxy
    Restart {
        #[command(flatten)]
        listen: Listen,
    },
    /// Show the proxy's status
    Status,
    /// Serve URLs without a port, on 443 (macOS)
    Install,
    /// Remove the 443 relay
    Uninstall,
    /// Run the proxy in the foreground
    #[command(hide = true)]
    Run {
        #[command(flatten)]
        listen: Listen,
    },
}

#[derive(clap::Args)]
pub struct Listen {
    /// HTTPS port (default: 1443) [env: PARCELA_HTTPS_PORT]
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub port: Option<u16>,
    /// Caddy binary.
    #[arg(long, default_value = "caddy", hide = true)]
    pub caddy: String,
}
