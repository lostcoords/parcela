//! Human renderings of command results. JSON output never passes through here.
use super::output::{Style, cell, table, url_of};
use serde_json::Value;

pub fn status(row: &Value, style: &Style) -> String {
    let workspace = &row["workspace"];
    let mut text = format!(
        "{} {}  {}\n  {}\n",
        style.bold(&cell(&workspace["project_id"])),
        cell(&workspace["label"]),
        style.state(&cell(&row["state"])),
        style.url(&url_of(row)),
    );
    let endpoints = row["live"]["endpoints"].as_object();
    let shared = |endpoint: &Value| -> Vec<String> {
        endpoint["shared"]
            .as_array()
            .into_iter()
            .flatten()
            .map(cell)
            .collect()
    };
    if let Some(endpoints) = endpoints.filter(|endpoints| endpoints.len() > 1) {
        let rows: Vec<Vec<String>> = endpoints
            .iter()
            .flat_map(|(service, endpoint)| {
                let state = if endpoint["http_ready"] == true {
                    style.state("ready")
                } else {
                    style.state("unhealthy")
                };
                std::iter::once(vec![
                    format!("  {service}"),
                    state,
                    cell(&endpoint["public_url"]),
                ])
                .chain(
                    shared(endpoint)
                        .into_iter()
                        .map(|url| vec![String::new(), String::new(), url]),
                )
            })
            .collect();
        text.push_str(&table(&["  SERVICE", "HTTP", "URL"], &rows));
    } else {
        for url in endpoints
            .into_iter()
            .flat_map(|endpoints| endpoints.values())
            .flat_map(shared)
        {
            text.push_str(&format!("  {}\n", style.url(&url)));
        }
    }
    let processes: Vec<Vec<String>> = row["processes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|process| {
            vec![
                format!("  {}", cell(&process["name"])),
                style.state(&cell(&process["state"])),
                cell(&process["pid"]),
            ]
        })
        .collect();
    if !processes.is_empty() {
        text.push_str(&table(&["  PROCESS", "STATE", "PID"], &processes));
    }
    let session = &row["recorded_session"];
    if session["status"] != "stopped" {
        text.push_str(&ports(&session["ports"]));
    }
    text.push_str(&format!(
        "  {}\n",
        style.dim(&format!("path {}", cell(&workspace["source"])))
    ));
    text
}

/// One line per named port: the name, its number and the variable that carries it.
fn ports(value: &Value) -> String {
    let ports = value.as_object().into_iter().flatten();
    let width = ports
        .clone()
        .map(|(name, _)| name.len() + 2)
        .max()
        .unwrap_or(0)
        .max(10);
    ports
        .map(|(name, port)| {
            format!(
                "  {name:<width$}{:<7}{}\n",
                cell(port),
                parcela::config::port_variable(name)
            )
        })
        .collect()
}

pub fn context(value: &Value, style: &Style) -> String {
    let workspace = &value["workspace"];
    let mut text = format!(
        "{} {}  {}\n",
        style.bold(&cell(&workspace["project_id"])),
        cell(&workspace["label"]),
        cell(&workspace["role"]),
    );
    if let Some(urls) = value["planned_urls"].as_object() {
        for (service, url) in urls {
            text.push_str(&format!("  {service:<10}{}\n", style.url(&cell(url))));
        }
    }
    text.push_str(&ports(&value["ports"]));
    text.push_str(&format!("  {:<10}{}\n", "path", cell(&workspace["source"])));
    text.push_str(&format!(
        "  {:<10}{}\n",
        "runtime",
        cell(&value["runtime_dir"])
    ));
    text.push_str(&format!("  {:<10}{}\n", "id", cell(&workspace["id"])));
    text
}

/// How people refer to an environment: the project for its primary checkout, else the preview name.
pub fn name(workspace: &Value) -> String {
    if workspace["role"] == "primary" {
        cell(&workspace["project_id"])
    } else {
        format!(
            "{} ({})",
            cell(&workspace["label"]),
            cell(&workspace["project_id"])
        )
    }
}

pub fn down(value: &Value, label: &str, style: &Style) -> String {
    if value["already_stopped"] == true {
        format!("{} {label} is not running\n", style.ok())
    } else {
        format!("{} Stopped {label}\n", style.ok())
    }
}

pub fn started(value: &Value, style: &Style) -> String {
    let url = cell(&value["session"]["public_url"]);
    let already = if value["already_running"] == true {
        " already"
    } else {
        ""
    };
    let mut text = if value["session"]["public_url"].is_null() {
        format!(
            "{} {} is{already} running (no HTTP services)\n",
            style.ok(),
            name(&value["workspace"])
        )
    } else {
        format!(
            "{} {} is{already} running at {}\n",
            style.ok(),
            name(&value["workspace"]),
            style.url(&url)
        )
    };
    let live = &value["live"]["endpoints"];
    let (main, others): (Vec<&Value>, Vec<&Value>) = value["session"]["endpoints"]
        .as_array()
        .into_iter()
        .flatten()
        .partition(|endpoint| endpoint["public_url"].as_str() == Some(url.as_str()));
    for endpoint in main.into_iter().chain(others) {
        let service = cell(&endpoint["service"]);
        if endpoint["public_url"].as_str() != Some(url.as_str()) {
            text.push_str(&format!(
                "  {service:<10}{}\n",
                style.url(&cell(&endpoint["public_url"]))
            ));
        }
        for shared in live[&service]["shared"].as_array().into_iter().flatten() {
            text.push_str(&format!("  {:<10}{}\n", "", style.url(&cell(shared))));
        }
    }
    text.push_str(&format!(
        "\n  {}    show service output\n  {}  check health\n  {}    stop it\n",
        style.bold("parcela logs"),
        style.bold("parcela status"),
        style.bold("parcela down"),
    ));
    text
}

pub fn recovered(value: &Value, style: &Style) -> String {
    if value["state"] == "stopped" {
        format!("{} Stopped\n", style.ok())
    } else {
        format!(
            "{} Reattached at {}\n",
            style.ok(),
            style.url(&cell(&value["session"]["public_url"]))
        )
    }
}

pub fn proxy_status(value: &Value, style: &Style) -> String {
    let listening = cell(&value["listening"]);
    let mut text = if value["running"] == true {
        let already = if value["already_running"] == true {
            " (already running)"
        } else {
            ""
        };
        format!(
            "{} Proxy running{already}: {listening}\n  {:<9}{}\n  {:<9}{} (Caddy {})\n  {:<9}Parcela {}\n",
            style.ok(),
            "urls",
            style.url(&cell(&value["url"])),
            "pid",
            cell(&value["pid"]),
            cell(&value["caddy_pid"]),
            "version",
            cell(&value["version"]),
        )
    } else {
        let mut text = format!("{} Proxy not running ({listening})\n", style.bad());
        if let Some(error) = value["error"].as_str() {
            text.push_str(&format!("  {:<9}{error}\n", "problem"));
        }
        text.push_str(&format!("  {:<9}{}\n", "log", cell(&value["log"])));
        text.push_str(&format!(
            "  Start it with {}\n",
            style.bold("parcela proxy start")
        ));
        return text;
    };
    text.push_str(&format!("  {:<9}{}\n", "log", cell(&value["log"])));
    if let Some(warning) = value["version_warning"].as_str() {
        text.push_str(&format!("  {} {warning}\n", style.bad()));
    }
    if value["trusted"] != true {
        text.push_str(&format!(
            "  {}\n",
            style.dim("Run `parcela trust` to trust these URLs")
        ));
    }
    text
}

pub fn proxy_stopped(value: &Value, style: &Style) -> String {
    if value["stopped"] == true {
        format!("{} Stopped the proxy\n", style.ok())
    } else {
        format!("{} The proxy is not running\n", style.ok())
    }
}
