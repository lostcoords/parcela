use super::output::Output;
use anyhow::{Context, Result, bail};
use parcela::{project::config::Manifest, proxy::frontdoor::FrontDoor};
use std::{fs, io::Write, path::Path};

const DOCS: &str = "https://parcela.run/docs/configuration/";
const SCHEMA: &str = "https://parcela.run/schema/v1.json";

/// A guessed dev server command and the evidence it came from.
struct Suggestion {
    command: String,
    reason: Option<&'static str>,
}

fn suggest(directory: &Path) -> Suggestion {
    if let Some(script) = fs::read_to_string(directory.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|package| package["scripts"]["dev"].as_str().map(str::to_owned))
    {
        let run = if directory.join("pnpm-lock.yaml").is_file() {
            "pnpm run dev"
        } else if directory.join("yarn.lock").is_file() {
            "yarn dev"
        } else if directory.join("bun.lock").is_file() || directory.join("bun.lockb").is_file() {
            "bun run dev"
        } else {
            "npm run dev --"
        };
        let host = if script.contains("next") {
            "--hostname"
        } else {
            "--host"
        };
        return Suggestion {
            command: format!("{run} --port $PORT {host} 127.0.0.1"),
            reason: Some("package.json has a dev script; check that it accepts these flags"),
        };
    }
    if directory.join("Gemfile").is_file() && directory.join("bin/rails").is_file() {
        return Suggestion {
            command: "bin/rails server -p $PORT -b 127.0.0.1".into(),
            reason: Some("Rails app detected"),
        };
    }
    if directory.join("manage.py").is_file() {
        return Suggestion {
            command: "python3 manage.py runserver 127.0.0.1:$PORT".into(),
            reason: Some("Django manage.py detected"),
        };
    }
    Suggestion {
        command: "python3 -m http.server $PORT --bind 127.0.0.1".into(),
        reason: Some("No dev server detected; this serves the directory, replace it with yours"),
    }
}

/// Plain scalars stay readable; anything YAML could misread is single-quoted.
fn scalar(text: &str) -> String {
    let plain = !text.is_empty()
        && !text.contains(": ")
        && !text.contains(" #")
        && !text.ends_with(':')
        && !text.starts_with(|c: char| "-?:,[]{}#&*!|>'\"%@`~ ".contains(c))
        && ![
            "true", "false", "yes", "no", "on", "off", "y", "n", "null", "~",
        ]
        .contains(&text.to_ascii_lowercase().as_str());
    if plain {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "''"))
    }
}

pub fn run(
    directory: &Path,
    name: Option<&str>,
    command: Option<&str>,
    front_door: FrontDoor,
    out: Output,
) -> Result<()> {
    let directory = directory
        .canonicalize()
        .with_context(|| format!("resolve {}", directory.display()))?;
    let path = directory.join("parcela.yaml");
    if fs::symlink_metadata(&path).is_ok() {
        bail!(
            "{} already exists; edit it, or check it with `parcela doctor`",
            path.display()
        );
    }
    let suggestion = match command {
        Some(command) => Suggestion {
            command: command.to_owned(),
            reason: None,
        },
        None => suggest(&directory),
    };
    let project = match name {
        Some(name) => parcela::project::config::sanitize_label(name)
            .context("--name needs letters or digits for a DNS-safe project name")?,
        None => parcela::project::config::inferred_project_name(&directory)?,
    };
    let mut text = format!(
        "# yaml-language-server: $schema={SCHEMA}\n# Reference: {DOCS}\nversion: 1\nproject: {project}\n\nservices:\n  web:\n"
    );
    if let Some(reason) = suggestion.reason {
        text.push_str(&format!("    # {reason}\n"));
    }
    text.push_str(&format!(
        "    command: {}\n    http:\n      healthcheck: /\n",
        scalar(&suggestion.command)
    ));
    text.push_str(
        "\n# Commands that prepare a new environment, such as installing packages or creating a database:\n# hooks:\n#   setup:\n#     command: bin/setup\n",
    );
    let manifest = Manifest::parse(&text).context(
        "the generated parcela.yaml would be invalid; choose another --name or --command",
    )?;
    let url = front_door.url(&manifest.hostname(true, "primary")?);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(text.as_bytes())?;
    let result = serde_json::json!({
        "created": path,
        "project": manifest.project.name,
        "url": url,
        "services": {"web": suggestion.command},
    });
    out.emit(&result, |style| {
        format!(
            "{} Created {}\n  web  {}\n  url  {}\n\nNext: {}\n",
            style.ok(),
            path.display(),
            suggestion.command,
            style.url(&url),
            style.bold("parcela up"),
        )
    })
}
