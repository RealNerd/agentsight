use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::parser::session_index;
use crate::sanitize;

pub struct SanitizeArgs {
    pub identifier: String,
    pub output: Option<PathBuf>,
    pub max_lines: usize,
    pub verbose: bool,
}

pub fn run(claude_dir: &Path, args: &SanitizeArgs) -> Result<()> {
    let raw_path = resolve_input_path(claude_dir, &args.identifier, args.verbose)?;

    let file = File::open(&raw_path)?;
    let reader = BufReader::new(file);
    let mut ctx = sanitize::SanitizeContext::new();

    let mut out: Box<dyn Write> = match &args.output {
        Some(path) => Box::new(File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };

    let mut count = 0;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                if args.verbose {
                    eprintln!("warn: failed to read line: {}", e);
                }
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let sanitized = sanitize::sanitize_line(&mut ctx, &line);
        writeln!(out, "{}", sanitized)?;

        count += 1;
        if args.max_lines > 0 && count >= args.max_lines {
            break;
        }
    }

    if args.output.is_some() {
        eprintln!("Wrote {} sanitized lines to output", count);
    }

    Ok(())
}

/// Resolve the identifier to a concrete JSONL file path.
/// Accepts: direct .jsonl path, UUID prefix, or session slug.
fn resolve_input_path(claude_dir: &Path, identifier: &str, verbose: bool) -> Result<PathBuf> {
    // Direct path to a .jsonl file
    let as_path = PathBuf::from(identifier);
    if as_path.extension().and_then(|e| e.to_str()) == Some("jsonl") && as_path.exists() {
        return Ok(as_path);
    }

    // Try session discovery
    let sessions = session_index::discover_sessions(claude_dir)?;
    let session_refs: Vec<&session_index::SessionFile> = sessions.iter().collect();

    // UUID prefix match
    if let Some(sf) = session_refs
        .iter()
        .find(|sf| sf.session_id.starts_with(identifier))
    {
        return Ok(sf.path.clone());
    }

    // Slug match — need to parse to get slug, pick newest
    let id_lower = identifier.to_lowercase();
    let mut best: Option<(PathBuf, Option<chrono::DateTime<chrono::Utc>>)> = None;

    for sf in &session_refs {
        let entries = crate::parser::reader::parse_session_file(&sf.path, verbose)?;
        let project_path = crate::parser::reader::decode_project_path(&sf.project_dir_name);
        let summary =
            crate::parser::reader::summarize_session(&entries, sf.session_id.clone(), project_path);

        let matches = match summary.slug.as_deref() {
            Some(s) => {
                let s_lower = s.to_lowercase();
                s_lower == id_lower || s_lower.contains(&id_lower)
            }
            None => false,
        };

        if !matches {
            continue;
        }

        let is_newer = match (&best, summary.start_time) {
            (None, _) => true,
            (Some((_, prev_time)), Some(new_start)) => match prev_time {
                Some(prev) => new_start > *prev,
                None => true,
            },
            _ => false,
        };

        if is_newer {
            best = Some((sf.path.clone(), summary.start_time));
        }
    }

    best.map(|(path, _)| path)
        .ok_or_else(|| anyhow::anyhow!("No session found matching '{}'", identifier))
}
