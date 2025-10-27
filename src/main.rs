use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(
    name = "crawl",
    version,
    about = "Minimal C include-graph crawler (no toolchain)"
)]
struct Cli {
    /// Database directory (will store index as JSON)
    #[arg(global = true, long, default_value = ".dep_crawler")]
    db_dir: PathBuf,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Build/refresh the include graph and file hashes by scanning roots
    Scan {
        /// One or more root directories to scan
        #[arg(required = true)]
        roots: Vec<PathBuf>,
        /// Extra ignore globs (repeatable), e.g. "build/**"
        #[arg(long = "ignore", num_args = 0..)]
        ignores: Vec<String>,
    },
    /// Given changed paths, show the minimal set of .c files to recompile
    Impact {
        /// Paths that may have changed (files or directories)
        #[arg(required = true)]
        changed: Vec<PathBuf>,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Db {
    /// Absolute canonical roots scanned
    roots: Vec<String>,
    /// Map file -> sha256 hash of content (comment/whitespace stripped for headers)
    hash: BTreeMap<String, String>,
    /// Directed include edges: A (file) -> [B (included file), ...]
    edges: BTreeMap<String, Vec<String>>,
    /// Reverse edges (redundant but handy): B -> [A]
    rev: BTreeMap<String, Vec<String>>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    fs::create_dir_all(&cli.db_dir).ok();
    match &cli.cmd {
        Cmd::Scan { roots, ignores } => cmd_scan(&cli, roots, ignores.clone()),
        Cmd::Impact { changed } => cmd_impact(&cli, changed),
    }
}

fn cmd_scan(cli: &Cli, roots: &Vec<PathBuf>, ignores: Vec<String>) -> Result<()> {
    let roots_abs: Vec<PathBuf> = roots.into_iter().map(|r| canonicalize_lenient(r)).collect();

    let header_roots = discover_header_roots(&roots_abs);
    let ignore_set = default_ignores()
        .into_iter()
        .chain(ignores)
        .collect::<BTreeSet<_>>();

    let mut db = Db::default();
    db.roots = roots_abs
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    // First pass: collect files, store hashes
    let files: Vec<PathBuf> = walk_c_and_h(&roots_abs, &ignore_set).collect();
    for f in &files {
        let h = hash_for_storage(f)?;
        db.hash.insert(f.to_string_lossy().to_string(), h);
    }

    // Second pass: build include edges
    for f in &files {
        let text = read_to_string_lossy(f)?;
        let includes = parse_includes(&text);
        for inc in includes {
            if let Some(target) = resolve_include(f, &inc, &header_roots) {
                add_edge(&mut db, f, &target);
            }
        }
    }

    // Persist
    let out = cli.db_dir.join("minimal_index.json");
    fs::write(&out, serde_json::to_vec_pretty(&db)?)?;
    eprintln!(
        "Indexed {} files, {} edges → {}",
        db.hash.len(),
        db.edges.values().map(|v| v.len()).sum::<usize>(),
        out.display()
    );
    Ok(())
}

fn cmd_impact(cli: &Cli, changed_inputs: &Vec<PathBuf>) -> Result<()> {
    let db_path = cli.db_dir.join("minimal_index.json");
    let bytes =
        fs::read(&db_path).with_context(|| format!("read index at {}", db_path.display()))?;
    let db: Db = serde_json::from_slice(&bytes)?;

    // Expand directories to contained files we know about
    let mut changed_files = BTreeSet::new();
    for p in changed_inputs {
        let p = canonicalize_lenient(p);
        if p.is_dir() {
            for (f, _) in &db.hash {
                let fpath = PathBuf::from(f);
                if is_within(&fpath, &p) {
                    changed_files.insert(fpath);
                }
            }
        } else {
            changed_files.insert(p);
        }
    }

    // Detect which of these actually changed by hashing current contents
    let mut dirty: Vec<PathBuf> = Vec::new();
    for f in changed_files {
        let key = f.to_string_lossy().to_string();
        let new_hash = match hash_for_storage(&f) {
            Ok(h) => h,
            Err(_) => "<missing>".into(), // deleted files count as changed
        };
        let old_hash = db.hash.get(&key);
        if old_hash.map(|h| h != &new_hash).unwrap_or(true) {
            dirty.push(f);
        }
    }

    if dirty.is_empty() {
        println!("No content changes detected in known files.");
        return Ok(());
    }

    // Reverse BFS from each dirty node to all reachable .c TUs
    let mut need: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut q: VecDeque<String> = VecDeque::new();
    for d in dirty {
        q.push_back(d.to_string_lossy().to_string());
    }

    while let Some(node) = q.pop_front() {
        if !seen.insert(node.clone()) {
            continue;
        }
        if node.ends_with(".c") {
            need.insert(node.clone());
        }
        if let Some(parents) = db.rev.get(&node) {
            for p in parents {
                q.push_back(p.clone());
            }
        }
    }

    if need.is_empty() {
        println!("No translation units (.c) are downstream of the changed files.");
        return Ok(());
    }

    println!("Recompile {} translation unit(s):\n", need.len());
    for tu in &need {
        println!("  {}", tu);
    }

    Ok(())
}

// ---------------- include parsing & resolution -------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IncKind {
    Quoted,
    Angled,
}

#[derive(Debug, Clone)]
struct IncTok {
    kind: IncKind,
    path: String,
}

fn parse_includes(text: &str) -> Vec<IncTok> {
    let mut out = Vec::new();
    let stripped = strip_comments(text);
    for line in stripped.lines() {
        let l = line.trim_start();
        if !l.starts_with('#') {
            continue;
        }
        let l = l.strip_prefix('#').unwrap().trim_start();
        if !l.starts_with("include") {
            continue;
        }
        let rest = l["include".len()..].trim_start();
        if let Some(p) = rest.strip_prefix('\"') {
            // "..."
            if let Some(end) = p.find('\"') {
                out.push(IncTok {
                    kind: IncKind::Quoted,
                    path: p[..end].to_string(),
                });
            }
        } else if let Some(p) = rest.strip_prefix('<') {
            // <...>
            if let Some(end) = p.find('>') {
                out.push(IncTok {
                    kind: IncKind::Angled,
                    path: p[..end].to_string(),
                });
            }
        }
    }
    out
}

fn strip_comments(s: &str) -> String {
    // Remove /* ... */ and // ...
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let b = s.as_bytes();
    while i < b.len() {
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            // block
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
            // line
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn resolve_include(from: &Path, inc: &IncTok, header_roots: &Vec<PathBuf>) -> Option<PathBuf> {
    // We only resolve files that live within scanned roots; ignore system headers.
    let from_dir = from.parent().unwrap_or_else(|| Path::new("."));

    // 1) Quoted includes: try relative to including file first
    if inc.kind == IncKind::Quoted {
        let cand = canonicalize_lenient(&from_dir.join(&inc.path));
        if cand.exists() && is_under_any_root(&cand, header_roots) {
            return Some(cand);
        }
    }

    // 2) Try each header root joined with inc.path
    for root in header_roots {
        let cand = canonicalize_lenient(&root.join(&inc.path));
        if cand.exists() && is_under_any_root(&cand, header_roots) {
            return Some(cand);
        }
    }

    None // unresolved (treated as project-external)
}

fn discover_header_roots(roots: &Vec<PathBuf>) -> Vec<PathBuf> {
    // Heuristics: each root, and any immediate <root>/include directories
    let mut set: BTreeSet<PathBuf> = BTreeSet::new();
    for r in roots {
        set.insert(r.clone());
        let inc = r.join("include");
        if inc.is_dir() {
            set.insert(inc);
        }
    }
    set.into_iter().collect()
}

// ---------------- graph utils ------------------------------------------------

fn add_edge(db: &mut Db, a: &Path, b: &Path) {
    let a = a.to_string_lossy().to_string();
    let b = b.to_string_lossy().to_string();
    db.edges.entry(a.clone()).or_default().push(b.clone());
    db.rev.entry(b).or_default().push(a);
}

fn is_within(p: &Path, root: &Path) -> bool {
    let Ok(p) = p.canonicalize() else {
        return false;
    };
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    p.starts_with(root)
}

fn is_under_any_root(p: &Path, roots: &Vec<PathBuf>) -> bool {
    roots.iter().any(|r| is_within(p, r))
}

// ---------------- hashing & IO -----------------------------------------------

fn hash_for_storage(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)
        .with_context(|| format!("read {}", path.display()))?;
    let is_header = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e, "h" | "hh" | "hpp" | "hxx" | "inc"))
        .unwrap_or(false);
    let data = if is_header {
        strip_comments(&buf).into_bytes()
    } else {
        buf.into_bytes()
    };
    Ok(hex_sha256(&data))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn read_to_string_lossy(p: &Path) -> Result<String> {
    Ok(fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?)
}

fn canonicalize_lenient(p: &PathBuf) -> PathBuf {
    p.canonicalize().unwrap_or(p.clone())
}

fn walk_c_and_h<'a>(
    roots: &'a [PathBuf],
    ignores: &'a BTreeSet<String>,
) -> impl Iterator<Item = PathBuf> + 'a {
    roots.iter().flat_map(move |root| {
        WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(move |e| {
                let p = e.path();
                // Skip dirs and ignored patterns (simple contains match for now)
                if e.file_type().is_dir() {
                    return false;
                }
                let s = p.to_string_lossy();
                for ig in ignores {
                    if s.contains(ig) {
                        return false;
                    }
                }
                matches!(
                    p.extension().and_then(|x| x.to_str()),
                    Some("c" | "h" | "hpp" | "hh" | "hxx" | "inc")
                )
            })
            .map(|e| canonicalize_lenient(&e.path().to_path_buf()))
    })
}

fn default_ignores() -> Vec<String> {
    vec![
        ".git/".into(),
        "build/".into(),
        "cmake-build-".into(),
        "target/".into(),
        "node_modules/".into(),
        ".cache/".into(),
        ".deps/".into(),
        "out/".into(),
        "dist/".into(),
        "third_party/".into(), // keep it simple; you can override with --ignore "!third_party/**" later if you want
    ]
}
