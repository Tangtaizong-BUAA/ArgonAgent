//! Lightweight repository map builder.
//!
//! This is the first context-retrieval pass before targeted file reads. It is
//! read-only, skips high-noise and sensitive paths, and produces a compact map
//! suitable for model context and GUI project summaries.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMapRequest {
    pub root: PathBuf,
    pub max_files: usize,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMapResult {
    pub root: PathBuf,
    pub file_count: usize,
    pub omitted_count: usize,
    pub tech_stack: Vec<String>,
    pub important_files: Vec<PathBuf>,
    pub tree_lines: Vec<String>,
}

impl RepoMapResult {
    pub fn to_context_text(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("root: {}\n", self.root.to_string_lossy()));
        output.push_str(&format!(
            "files_seen: {} omitted: {}\n",
            self.file_count, self.omitted_count
        ));
        output.push_str(&format!("tech_stack: {}\n", self.tech_stack.join(", ")));
        output.push_str("important_files:\n");
        for file in &self.important_files {
            output.push_str(&format!("- {}\n", file.to_string_lossy()));
        }
        output.push_str("tree:\n");
        for line in &self.tree_lines {
            output.push_str(line);
            output.push('\n');
        }
        output
    }
}

pub fn build_repo_map(request: &RepoMapRequest) -> Result<RepoMapResult, String> {
    if !request.root.exists() {
        return Err("repo map root does not exist".to_string());
    }
    if !request.root.is_dir() {
        return Err("repo map root must be a directory".to_string());
    }
    let mut state = RepoMapState {
        root: request.root.clone(),
        max_files: request.max_files.max(1),
        max_depth: request.max_depth,
        file_count: 0,
        omitted_count: 0,
        tech_stack: BTreeSet::new(),
        important_files: Vec::new(),
        tree_lines: Vec::new(),
    };
    scan_dir(&mut state, &request.root, 0)?;
    Ok(RepoMapResult {
        root: request.root.clone(),
        file_count: state.file_count,
        omitted_count: state.omitted_count,
        tech_stack: state.tech_stack.into_iter().collect(),
        important_files: state.important_files,
        tree_lines: state.tree_lines,
    })
}

struct RepoMapState {
    root: PathBuf,
    max_files: usize,
    max_depth: usize,
    file_count: usize,
    omitted_count: usize,
    tech_stack: BTreeSet<String>,
    important_files: Vec<PathBuf>,
    tree_lines: Vec<String>,
}

fn scan_dir(state: &mut RepoMapState, dir: &Path, depth: usize) -> Result<(), String> {
    if depth > state.max_depth {
        state.omitted_count += 1;
        return Ok(());
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if state.file_count >= state.max_files {
            state.omitted_count += 1;
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip(&name, &path) {
            state.omitted_count += 1;
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            state.omitted_count += 1;
            continue;
        };
        if file_type.is_symlink() {
            state.omitted_count += 1;
            continue;
        }
        let relative = path
            .strip_prefix(&state.root)
            .unwrap_or(&path)
            .to_path_buf();
        let indent = "  ".repeat(depth);
        if file_type.is_dir() {
            state
                .tree_lines
                .push(format!("{}{}/", indent, relative.to_string_lossy()));
            scan_dir(state, &path, depth + 1)?;
        } else if file_type.is_file() {
            state.file_count += 1;
            state
                .tree_lines
                .push(format!("{}{}", indent, relative.to_string_lossy()));
            detect_tech_stack(&mut state.tech_stack, &relative);
            if is_important_file(&relative) {
                state.important_files.push(relative);
            }
        }
    }
    Ok(())
}

fn should_skip(name: &str, path: &Path) -> bool {
    matches!(
        name,
        ".git"
            | "target"
            | "node_modules"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".researchcode"
            | "runs"
            | "dist"
            | "build"
            | ".DS_Store"
    ) || name.starts_with(".env")
        || path.components().any(|component| {
            let part = component.as_os_str().to_string_lossy();
            part == ".ssh" || part == ".gnupg"
        })
}

fn detect_tech_stack(tech_stack: &mut BTreeSet<String>, path: &Path) {
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    match file_name.as_ref() {
        "Cargo.toml" => {
            tech_stack.insert("rust".to_string());
        }
        "package.json" => {
            tech_stack.insert("node".to_string());
        }
        "pyproject.toml" | "requirements.txt" => {
            tech_stack.insert("python".to_string());
        }
        "tauri.conf.json" => {
            tech_stack.insert("tauri".to_string());
        }
        _ => {}
    }
    match path.extension().and_then(|value| value.to_str()) {
        Some("rs") => {
            tech_stack.insert("rust".to_string());
        }
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") => {
            tech_stack.insert("typescript/javascript".to_string());
        }
        Some("py") => {
            tech_stack.insert("python".to_string());
        }
        Some("md") => {
            tech_stack.insert("markdown/docs".to_string());
        }
        Some("sql") => {
            tech_stack.insert("sqlite/sql".to_string());
        }
        _ => {}
    }
}

fn is_important_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        file_name,
        "AGENTS.md"
            | "README.md"
            | "Cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "requirements.txt"
            | "tsconfig.json"
            | "tauri.conf.json"
            | "sqlite_schema_v0.sql"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn builds_compact_repo_map_and_skips_sensitive_noise() {
        let root = temp_root("repo-map");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname='demo'\n").unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
        fs::write(root.join(".env"), "SECRET=1\n").unwrap();
        fs::write(root.join(".git/config"), "ignored\n").unwrap();
        let result = build_repo_map(&RepoMapRequest {
            root: root.clone(),
            max_files: 20,
            max_depth: 4,
        })
        .unwrap();
        assert!(result.tech_stack.contains(&"rust".to_string()));
        assert!(result
            .important_files
            .iter()
            .any(|path| path == Path::new("Cargo.toml")));
        let text = result.to_context_text();
        assert!(text.contains("src/lib.rs"));
        assert!(!text.contains(".env"));
        assert!(!text.contains(".git/config"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn enforces_file_limit() {
        let root = temp_root("repo-map-limit");
        fs::write(root.join("a.rs"), "").unwrap();
        fs::write(root.join("b.rs"), "").unwrap();
        let result = build_repo_map(&RepoMapRequest {
            root: root.clone(),
            max_files: 1,
            max_depth: 1,
        })
        .unwrap();
        assert_eq!(result.file_count, 1);
        assert!(result.omitted_count >= 1);
        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-{name}-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
