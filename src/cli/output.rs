use anyhow::Result;
use parcela::paint::Paint;
use serde_json::Value;
use std::io::{IsTerminal, Write};

/// Where command results go: aligned text for people, JSON for agents and pipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Human,
    Json,
}

impl Output {
    pub fn detect(json: bool) -> Self {
        if !json && std::io::stdout().is_terminal() {
            Self::Human
        } else {
            Self::Json
        }
    }

    /// Print one command result. Diagnostics belong on stderr, never here.
    pub fn emit(
        self,
        value: &impl serde::Serialize,
        human: impl FnOnce(&Style) -> String,
    ) -> Result<()> {
        let mut out = std::io::stdout().lock();
        match self {
            Self::Json => {
                serde_json::to_writer_pretty(&mut out, value)?;
                writeln!(out)?;
            }
            Self::Human => {
                let text = human(&Style::detect());
                write!(out, "{text}")?;
                if !text.ends_with('\n') {
                    writeln!(out)?;
                }
            }
        }
        Ok(())
    }
}

pub struct Style(Paint);

impl Style {
    fn detect() -> Self {
        Self(Paint::stdout())
    }
    fn paint(&self, code: &str, text: &str) -> String {
        self.0.paint(code, text)
    }
    pub fn ok(&self) -> String {
        self.paint("32", "✓")
    }
    pub fn bad(&self) -> String {
        self.paint("31", "✗")
    }
    pub fn url(&self, text: &str) -> String {
        self.paint("1;36", text)
    }
    pub fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }
    pub fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }
    pub fn state(&self, state: &str) -> String {
        match state {
            "ready" | "running" => self.paint("32", state),
            "unhealthy" | "unknown" | "error" => self.paint("31", state),
            "starting" | "stopping" => self.paint("33", state),
            _ => self.dim(state),
        }
    }
}

/// Render a value as a single table cell: strings unquoted, control characters blanked.
pub fn cell(value: &Value) -> String {
    let text = match value {
        Value::Null => "-".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// Left-aligned columns separated by two spaces. Widths ignore ANSI color codes.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    fn width(text: &str) -> usize {
        let mut visible = 0;
        let mut escape = false;
        for character in text.chars() {
            match (escape, character) {
                (false, '\x1b') => escape = true,
                (true, 'm') => escape = false,
                (true, _) => {}
                (false, _) => visible += 1,
            }
        }
        visible
    }
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(width(value));
        }
    }
    let line = |values: Vec<&str>| {
        let mut text = String::new();
        for (index, value) in values.iter().enumerate() {
            text.push_str(value);
            if index + 1 < values.len() {
                text.push_str(&" ".repeat(widths[index] - width(value) + 2));
            }
        }
        text.push('\n');
        text
    };
    let mut text = line(headers.to_vec());
    for row in rows {
        text.push_str(&line(row.iter().map(String::as_str).collect()));
    }
    text
}

/// One line per environment row from `lifecycle::status`.
pub fn status_rows(rows: &[Value], style: &Style) -> String {
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            let workspace = &row["workspace"];
            vec![
                cell(&workspace["project_id"]),
                format!(
                    "{}{}",
                    cell(&workspace["label"]),
                    if row["current"] == true { " *" } else { "" }
                ),
                style.state(&cell(&row["state"])),
                url_of(row),
                cell(&workspace["source"]),
            ]
        })
        .collect();
    if rows.is_empty() {
        return "No environments registered. Start one with `parcela up`.\n".into();
    }
    table(&["PROJECT", "NAME", "STATE", "URL", "PATH"], &rows)
}

/// The URL `lifecycle::status` resolved for the row.
pub fn url_of(row: &Value) -> String {
    cell(&row["url"])
}

/// Diagnostics for status rows go to stderr so stdout stays a clean table.
pub fn status_diagnostics(rows: &[Value]) {
    for row in rows {
        if let Some(diagnostic) = row["diagnostic"].as_str() {
            eprintln!(
                "parcela: {}: {diagnostic}",
                cell(&row["workspace"]["label"])
            );
        }
    }
}
