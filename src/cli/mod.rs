mod args;
mod autostart;
mod commands;
mod config;
mod doctor;
mod environments;
mod init;
mod inventory;
mod output;
mod proxy;
mod render;
mod telemetry;
mod upgrade;

use clap::Parser;

pub fn main() {
    let args = args::Args::parse();
    if let Err(error) = upgrade::prepare(&args) {
        let paint = parcela::paint::Paint::stderr();
        eprintln!(
            "{} {}",
            paint.paint("1;31", "parcela:"),
            paint.commands(&format!("{error:#}"))
        );
        std::process::exit(1);
    }
    let session = telemetry::begin(args.global.state_dir.clone());
    let result = match args.command {
        Some(command) => commands::run(args::Cli::new(args.global, command)),
        None => commands::bare(args.global),
    };
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            let paint = parcela::paint::Paint::stderr();
            eprintln!(
                "{} {}",
                paint.paint("1;31", "parcela:"),
                paint.commands(&format!("{error:#}"))
            );
            1
        }
    };
    session.finish(code);
    std::process::exit(code);
}
