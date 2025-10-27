use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Minimal file finder: lists relevant C/C++ header/source files.
#[derive(Parser, Debug)]
#[command(name = "crawl", version, about = "List relevant C files from roots")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Represents a mapping of include paths to their corresponding files.
#[derive(Debug)]
struct IncludeMapping {
    inner: HashMap<PathBuf, HashSet<PathBuf>>,
}

/// Represents a mapping of include paths to their corresponding files.
impl IncludeMapping {
    fn new() -> Self {
        IncludeMapping {
            inner: HashMap::new(),
        }
    }
    fn insert(&mut self, key: PathBuf, value: PathBuf) {
        self.inner.entry(key).or_default().insert(value);
    }
}

impl fmt::Display for IncludeMapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (key, value) in &self.inner {
            writeln!(f, "{}: {:?}", key.display(), value)?;
        }
        Ok(())
    }
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Recursively list relevant files under given roots
    List {
        /// One or more root directories to scan
        #[arg(required = true)]
        roots: Vec<PathBuf>,

        /// Repeatable ignore patterns (substring match), e.g. --ignore build/ --ignore .git/
        #[arg(long = "ignore", value_name = "PATTERN", num_args = 0..)]
        ignores: Vec<String>,

        /// Override relevant file extensions (comma-separated, no dots). Default: c,h,hh,hpp,hxx,inc
        #[arg(long = "exts", value_name = "CSV")]
        exts: Option<String>,

        /// Follow symlinks during traversal
        #[arg(long)]
        follow_symlinks: bool,
    },

    Scan {
        /// One or more root directories to scan
        #[arg(required = true)]
        roots: Vec<PathBuf>,

        /// Repeatable ignore patterns (substring match), e.g. --ignore build/ --ignore .git/
        #[arg(long = "ignore", value_name = "PATTERN", num_args = 0..)]
        ignores: Vec<String>,

        /// Override relevant file extensions (comma-separated, no dots). Default: c,h,hh,hpp,hxx,inc
        #[arg(long = "exts", value_name = "CSV")]
        exts: Option<String>,

        /// Follow symlinks during traversal
        #[arg(long)]
        follow_symlinks: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::List {
            roots,
            ignores,
            exts,
            follow_symlinks,
        } => cmd_list(roots, ignores, exts, follow_symlinks),
        Cmd::Scan {
            roots,
            ignores,
            exts,
            follow_symlinks,
        } => cmd_scan(roots, ignores, exts, follow_symlinks),
    }
}

fn cmd_scan(
    roots: Vec<PathBuf>,
    ignores: Vec<String>,
    exts_csv: Option<String>,
    follow_symlinks: bool,
) -> Result<()> {
    let mut mapping = IncludeMapping::new();
    let found = list_relevant_files(roots, ignores, exts_csv, follow_symlinks)?;
    for path in found {
        find_include_lines(&path, &mut mapping)?;
    }

    println!("{:#?}", mapping);
    Ok(())
}

fn find_include_lines(path: &Path, mapping: &mut IncludeMapping) -> Result<()> {
    let file =
        File::open(path).with_context(|| format!("failed to open file {}", path.display()))?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim_start();
        if trimmed.starts_with("#include") {
            let parsed = parse_include_path(&trimmed);
            if let Some(include) = parsed {
                mapping.insert(include, PathBuf::from(path));
            }
        }
    }
    Ok(())
}

/// Parses an `#include` line like `#include "../thingy/thing.c"`
/// and returns `Some(PathBuf)` for quoted includes.
/// Returns `None` for angle-bracket includes or invalid syntax.
fn parse_include_path(line: &str) -> Option<PathBuf> {
    // Slice off "#include"
    let rest = line["#include".len()..].trim_start();

    if rest.starts_with('<') {
        // System include — ignore
        return None;
    }

    if let Some(start) = rest.find('"') {
        let after_start = &rest[start + 1..];
        if let Some(end) = after_start.find('"') {
            let path_str = &after_start[..end];
            // Normalize path separators if needed
            let path = PathBuf::from(path_str);
            return Some(path);
        }
    }

    None
}

///Lists all the relevant files found under a given root directory
fn cmd_list(
    roots: Vec<PathBuf>,
    ignores: Vec<String>,
    exts_csv: Option<String>,
    follow_symlinks: bool,
) -> Result<()> {
    let mut found = list_relevant_files(roots, ignores, exts_csv, follow_symlinks)?;

    found.sort();
    found.dedup();
    for p in found {
        println!("{}", p.display());
    }

    Ok(())
}

fn list_relevant_files(
    roots: Vec<PathBuf>,
    ignores: Vec<String>,
    exts_csv: Option<String>,
    follow_symlinks: bool,
) -> Result<Vec<PathBuf>> {
    if roots.is_empty() {
        return Err(anyhow!("provide at least one root directory"));
    }

    let ignored = ignores.into_iter().collect::<BTreeSet<_>>();
    let exts = parse_exts(exts_csv);

    let mut found: Vec<PathBuf> = Vec::new();

    for root in roots {
        let root = canonicalize_lenient(&root);
        if !root.exists() {
            eprintln!("warn: skipping non-existent root {}", root.display());
            continue;
        }
        let walker = if follow_symlinks {
            WalkDir::new(&root).follow_links(true)
        } else {
            WalkDir::new(&root)
        };

        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();

            // skip directories
            if entry.file_type().is_dir() {
                continue;
            }

            // apply simple substring ignores
            let s = path.to_string_lossy();
            if ignored.iter().any(|pat| s.contains(pat)) {
                continue;
            }

            // filter by extension set
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if !exts.contains(ext) {
                    continue;
                }
            } else {
                // no extension → skip
                continue;
            }

            found.push(canonicalize_lenient(path));
        }
    }
    Ok(found)
}

fn parse_exts(exts_csv: Option<String>) -> BTreeSet<String> {
    let default = "c,h,hh,hpp,hxx,inc";
    let raw = exts_csv.as_deref().unwrap_or(default);
    raw.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().trim_start_matches('.').to_string())
        .collect()
}

fn canonicalize_lenient(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}
