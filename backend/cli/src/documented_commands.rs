//! Every `chakramcp …` command line in the agent skill, llms.txt and the docs
//! pages parses with the CLI's own parser (MCP and discovery spec §4.6), so
//! the docs can't drift from the CLI.
//!
//! Only lines in code blocks count: fenced blocks in Markdown, template-literal
//! blocks in the docs pages. Placeholders (`<call_id>`, `"$AGENT_ID"`) parse as
//! values; `[optional parts]` and `…` are dropped first. A line ending in
//! `# not-a-command` is skipped.

use std::path::{Path, PathBuf};

use clap::Parser;

use crate::Cli;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn docs_pages(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            docs_pages(&path, out);
        } else if path.file_name().is_some_and(|n| n == "page.tsx") {
            out.push(path);
        }
    }
}

/// Command lines in a file, with their line numbers. Shell continuations
/// (`\` at the end) are joined.
fn command_lines(path: &Path) -> Vec<(usize, String)> {
    let text = std::fs::read_to_string(path).unwrap();
    let markdown = path.extension().is_some_and(|e| e == "md" || e == "txt");
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut pending: Option<(usize, String)> = None;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if markdown && line.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if let Some((n, mut acc)) = pending.take() {
            acc.push(' ');
            acc.push_str(line.trim_end_matches('\\').trim());
            if line.ends_with('\\') {
                pending = Some((n, acc));
            } else {
                out.push((n, acc));
            }
            continue;
        }
        let candidate = if markdown {
            in_fence.then_some(line)
        } else {
            // A docs page's code block: a template literal's lines.
            Some(line.trim_start_matches("<code>{`"))
        };
        let Some(c) = candidate else { continue };
        let c = c.trim_start_matches("$ ");
        if !c.starts_with("chakramcp ") {
            continue;
        }
        if c.ends_with('\\') {
            pending = Some((i + 1, c.trim_end_matches('\\').trim().to_owned()));
        } else {
            out.push((i + 1, c.to_owned()));
        }
    }
    out
}

/// The words a command line passes to the CLI, after cutting what a shell
/// would and what's only notation.
fn words(line: &str) -> Option<Vec<String>> {
    if line.trim_end().ends_with("# not-a-command") {
        return None;
    }
    let mut cmd = line.to_owned();
    for cut in [
        " | ", " > ", " >> ", " 2>", " && ", " || ", " # ", "`}", "</code>",
    ] {
        if let Some(i) = cmd.find(cut) {
            cmd.truncate(i);
        }
    }
    // `$(…)` becomes a placeholder value.
    while let (Some(a), Some(b)) = (cmd.find("$("), cmd.find(')')) {
        if b < a {
            break;
        }
        cmd.replace_range(a..=b, "X");
    }
    // `[optional]` notation goes.
    while let (Some(a), Some(b)) = (cmd.find(" ["), cmd.find(']')) {
        if b < a {
            break;
        }
        cmd.replace_range(a..=b, "");
    }
    let cmd = cmd
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    // A `<placeholder with spaces>` is one value.
    let mut joined = String::with_capacity(cmd.len());
    let mut depth = 0;
    for ch in cmd.chars() {
        match ch {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            ' ' if depth > 0 => {
                joined.push('_');
                continue;
            }
            _ => {}
        }
        joined.push(ch);
    }
    // An elided value (`--name …`) stays a value; an elided rest goes.
    let mut w: Vec<String> = Vec::new();
    for t in shlex::split(&joined)? {
        if t == "…" || t == "..." {
            if w.last().is_some_and(|p| p.starts_with("--")) {
                w.push("X".into());
            }
            continue;
        }
        w.push(t);
    }
    Some(w)
}

#[test]
fn every_documented_command_parses() {
    let root = repo();
    let mut files = vec![
        root.join(".claude/skills/chakramcp-agent/SKILL.md"),
        root.join("frontend/public/llms.txt"),
    ];
    docs_pages(&root.join("frontend/src/app/(site)/docs"), &mut files);

    let mut checked = 0;
    let mut broken = Vec::new();
    for file in &files {
        for (n, line) in command_lines(file) {
            let Some(w) = words(&line) else { continue };
            checked += 1;
            if let Err(e) = Cli::try_parse_from(&w) {
                use clap::error::ErrorKind::*;
                let elided = line.contains('…') || line.contains("...");
                match e.kind() {
                    DisplayHelp | DisplayVersion => continue,
                    // A line shortened with `…` may leave out required parts.
                    MissingRequiredArgument if elided => continue,
                    _ => {}
                }
                let why = e.to_string();
                let first = why.lines().next().unwrap_or("").to_owned();
                broken.push(format!(
                    "{}:{n}: {line}\n    {first}",
                    file.strip_prefix(&root).unwrap_or(file).display()
                ));
            }
        }
    }
    assert!(checked > 50, "only {checked} documented commands found");
    assert!(
        broken.is_empty(),
        "{} documented commands don't parse:\n{}",
        broken.len(),
        broken.join("\n")
    );
}
