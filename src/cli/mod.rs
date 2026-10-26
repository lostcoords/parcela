mod args;
mod commands;
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
    let session = telemetry::begin(args.global.state_dir.clone());
    let result = match args.command {
        Some(command) => commands::run(args::Cli::new(args.global, command)),
        None => commands::bare(args.global),
    };
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!(
                "{} {error:#}",
                parcela::paint::Paint::stderr().paint("1;31", "parcela:")
            );
            1
        }
    };
    session.finish(code);
    std::process::exit(code);
}
