use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::patch::stable_text_hash;
use crate::tcml::ParsedToolArguments;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupeOutcome {
    FirstObservation,
    DuplicateExactKey { key: String, prior_seen_count: u32 },
    DuplicateCoveredBy { key: String, covering_key: String },
    DuplicateRateLimited { key: String, attempts: u32 },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ObservationCache {
    seen: BTreeMap<String, usize>,
    file_read_ranges: BTreeMap<String, Vec<ObservedFileReadRange>>,
    file_read_attempts: BTreeMap<String, usize>,
    file_mtimes: BTreeMap<String, u128>,
}

impl ObservationCache {
    /// Returns the number of distinct keys added this call (0 or 1).
    pub fn distinct_key_count(&self) -> usize {
        self.seen.len()
    }

    /// Invalidate all cached observations for a file path if its mtime has changed.
    /// Returns true if the file was modified (cache entries were cleared).
    pub fn invalidate_if_modified(&mut self, path: &str, current_mtime: u128) -> bool {
        if let Some(&cached_mtime) = self.file_mtimes.get(path) {
            if cached_mtime != current_mtime {
                let prefix = format!("file.read:{path}:");
                self.seen.retain(|key, _| !key.starts_with(&prefix));
                self.file_read_ranges.remove(path);
                self.file_read_attempts.remove(path);
                self.file_mtimes.insert(path.to_string(), current_mtime);
                return true;
            }
        }
        false
    }

    /// Record the mtime for a file path, to be checked on subsequent reads.
    pub fn record_mtime(&mut self, path: &str, mtime: u128) {
        self.file_mtimes.insert(path.to_string(), mtime);
    }

    pub fn contains_in_workspace(
        &mut self,
        tool_id: &str,
        arguments: &ParsedToolArguments,
        workspace_root: &Path,
    ) -> bool {
        self.invalidate_file_read_if_modified(tool_id, arguments, workspace_root);
        self.contains(tool_id, arguments)
    }

    pub fn check_and_record_in_workspace(
        &mut self,
        tool_id: &str,
        arguments: &ParsedToolArguments,
        workspace_root: &Path,
    ) -> Option<String> {
        let mtime = self.invalidate_file_read_if_modified(tool_id, arguments, workspace_root);
        let duplicate = self.check_and_record(tool_id, arguments);
        if duplicate.is_none() && tool_id == "file.read" {
            if let (Some(path), Some(mtime)) = (normalized_observation_path(arguments), mtime) {
                self.record_mtime(&path, mtime);
            }
        }
        duplicate
    }

    pub fn check_and_record(
        &mut self,
        tool_id: &str,
        arguments: &ParsedToolArguments,
    ) -> Option<String> {
        if tool_id == "file.read" {
            return self.check_and_record_file_read_dedupe_key(arguments);
        }
        let key = observation_key(tool_id, arguments)?;
        self.check_and_record_key(key)
    }

    /// Returns DedupeOutcome with stable keys (no attempts=N salt).
    pub fn check_and_record_with_outcome(
        &mut self,
        tool_id: &str,
        arguments: &ParsedToolArguments,
    ) -> Option<DedupeOutcome> {
        if tool_id == "file.read" {
            return self.check_and_record_file_read_outcome(arguments);
        }
        let key = observation_key(tool_id, arguments)?;
        let count = self.seen.entry(key.clone()).or_insert(0);
        if *count > 0 {
            *count += 1;
            Some(DedupeOutcome::DuplicateExactKey {
                key,
                prior_seen_count: (*count - 1) as u32,
            })
        } else {
            *count = 1;
            None
        }
    }

    pub fn check_and_record_key(&mut self, key: String) -> Option<String> {
        let count = self.seen.entry(key.clone()).or_insert(0);
        if *count > 0 {
            *count += 1;
            Some(key)
        } else {
            *count = 1;
            None
        }
    }

    pub fn contains(&self, tool_id: &str, arguments: &ParsedToolArguments) -> bool {
        if tool_id == "file.read" {
            return observation_key(tool_id, arguments)
                .map(|key| self.seen.contains_key(&key))
                .unwrap_or(false)
                || self.covering_file_read_key(arguments).is_some();
        }
        observation_key(tool_id, arguments)
            .map(|key| self.seen.contains_key(&key))
            .unwrap_or(false)
    }

    pub fn seen_count(&self, key: &str) -> usize {
        self.seen.get(key).copied().unwrap_or(0)
    }

    pub fn check_and_record_weak_hint(
        &mut self,
        tool_id: &str,
        arguments: &ParsedToolArguments,
    ) -> Option<String> {
        if observation_key(tool_id, arguments).is_some() {
            return None;
        }
        let key = weak_observation_key(tool_id, arguments);
        let count = self.seen.entry(key.clone()).or_insert(0);
        if *count > 0 {
            *count += 1;
            Some(key)
        } else {
            *count = 1;
            None
        }
    }

    /// Stable-key variant: returns the dedupe key without attempts=N salt.
    fn check_and_record_file_read_dedupe_key(
        &mut self,
        arguments: &ParsedToolArguments,
    ) -> Option<String> {
        let outcome = self.check_and_record_file_read_outcome(arguments)?;
        Some(match &outcome {
            DedupeOutcome::DuplicateExactKey { key, .. } => key.clone(),
            DedupeOutcome::DuplicateCoveredBy { covering_key, .. } => covering_key.clone(),
            DedupeOutcome::DuplicateRateLimited { key, .. } => key.clone(),
            DedupeOutcome::FirstObservation => unreachable!(),
        })
    }

    fn check_and_record_file_read_outcome(
        &mut self,
        arguments: &ParsedToolArguments,
    ) -> Option<DedupeOutcome> {
        let key = observation_key("file.read", arguments)?;
        let path = normalized_observation_path(arguments)?;

        // Exact key match first
        if let Some(count) = self.seen.get_mut(&key) {
            let prior = *count as u32;
            *count += 1;
            return Some(DedupeOutcome::DuplicateExactKey {
                key,
                prior_seen_count: prior,
            });
        }
        // Covered-by check
        if let Some(covered_by) = self.covering_file_read_key(arguments) {
            self.seen.insert(key.clone(), 1);
            return Some(DedupeOutcome::DuplicateCoveredBy {
                key,
                covering_key: covered_by,
            });
        }
        // Plan-like budget: track independently, return stable key
        if plan_like_path(&path) {
            let attempts = *self.file_read_attempts.entry(path.clone()).or_insert(0);
            if attempts >= 4 {
                self.file_read_attempts
                    .insert(path.clone(), attempts.saturating_add(1));
                self.seen.insert(key.clone(), 1);
                return Some(DedupeOutcome::DuplicateRateLimited {
                    key,
                    attempts: attempts as u32 + 1,
                });
            }
        }
        // First observation
        self.seen.insert(key.clone(), 1);
        if let Some(range) = ObservedFileReadRange::from_arguments(arguments, key) {
            self.file_read_ranges
                .entry(range.path.clone())
                .or_default()
                .push(range);
        }
        *self.file_read_attempts.entry(path.clone()).or_insert(0) += 1;
        // Mtime recorded by caller after successful observation.
        None
    }

    fn covering_file_read_key(&self, arguments: &ParsedToolArguments) -> Option<String> {
        let request = ObservedFileReadRange::from_arguments(arguments, String::new())?;
        self.file_read_ranges
            .get(&request.path)?
            .iter()
            .find(|observed| observed.covers(&request))
            .map(|observed| {
                format!(
                    "file.read:{}:covered_by={}",
                    request.path, observed.cache_key
                )
            })
    }

    fn invalidate_file_read_if_modified(
        &mut self,
        tool_id: &str,
        arguments: &ParsedToolArguments,
        workspace_root: &Path,
    ) -> Option<u128> {
        if tool_id != "file.read" {
            return None;
        }
        let path = normalized_observation_path(arguments)?;
        let mtime = file_read_mtime_ns(arguments, workspace_root)?;
        self.invalidate_if_modified(&path, mtime);
        Some(mtime)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedFileReadRange {
    path: String,
    start: usize,
    end_exclusive: Option<usize>,
    max_bytes: usize,
    cache_key: String,
}

impl ObservedFileReadRange {
    fn from_arguments(arguments: &ParsedToolArguments, cache_key: String) -> Option<Self> {
        let path = normalized_observation_path(arguments)?;
        let start = arguments.offset.unwrap_or(0);
        let end_exclusive = arguments.limit.map(|limit| start.saturating_add(limit));
        let max_bytes = arguments.max_bytes.unwrap_or(8_000).clamp(1, 80_000);
        Some(Self {
            path,
            start,
            end_exclusive,
            max_bytes,
            cache_key,
        })
    }

    fn covers(&self, request: &Self) -> bool {
        if self.max_bytes < request.max_bytes || self.start > request.start {
            return false;
        }
        match (self.end_exclusive, request.end_exclusive) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(observed_end), Some(request_end)) => observed_end >= request_end,
        }
    }
}

pub fn observation_key(tool_id: &str, arguments: &ParsedToolArguments) -> Option<String> {
    match tool_id {
        "file.read" => {
            let path = normalized_observation_path(arguments)?;
            let offset = arguments.offset.unwrap_or(0);
            let limit = arguments
                .limit
                .map(|value| value.to_string())
                .unwrap_or_else(|| "all".to_string());
            let max_bytes = bucket_file_read_max_bytes(arguments.max_bytes.unwrap_or(8_000));
            Some(format!(
                "file.read:{path}:offset={offset}:limit={limit}:max_bytes={max_bytes}"
            ))
        }
        "file.list_directory" => {
            let path = normalized_observation_path(arguments).unwrap_or_else(|| ".".to_string());
            let include_hidden = arguments.include_hidden.unwrap_or(false);
            let max_entries = arguments.max_results.unwrap_or(200).clamp(1, 2_000);
            Some(format!(
                "file.list_directory:{path}:hidden={include_hidden}:max_entries={max_entries}"
            ))
        }
        "file.list_tree" => {
            let path = normalized_observation_path(arguments).unwrap_or_else(|| ".".to_string());
            let depth = arguments.max_depth.unwrap_or(2).clamp(1, 6);
            let max_entries = arguments.max_results.unwrap_or(240).clamp(1, 2_000);
            Some(format!(
                "file.list_tree:{path}:depth={depth}:max_entries={max_entries}"
            ))
        }
        "repo.map" => {
            let path = normalized_observation_root(arguments).unwrap_or_else(|| ".".to_string());
            let max_files = arguments.max_files.unwrap_or(160).clamp(1, 400);
            let max_depth = arguments.max_depth.unwrap_or(4).clamp(1, 8);
            Some(format!(
                "repo.map:{path}:max_files={max_files}:max_depth={max_depth}"
            ))
        }
        "git.status" => Some("git.status".to_string()),
        "search.ripgrep" => {
            let pattern = arguments.pattern.as_deref()?.trim();
            if pattern.is_empty() {
                return None;
            }
            let (target_kind, target) = normalized_search_observation_target(arguments)
                .unwrap_or(("path", ".".to_string()));
            let max_results = arguments.max_results.unwrap_or(20).clamp(1, 100);
            Some(format!(
                "search.ripgrep:{target_kind}={target}:pattern={pattern}:max_results={max_results}"
            ))
        }
        _ => None,
    }
}

pub fn weak_observation_key(tool_id: &str, arguments: &ParsedToolArguments) -> String {
    let mut fields = Vec::new();
    push_arg(&mut fields, "path", arguments.path.clone());
    push_arg(&mut fields, "root", arguments.root.clone());
    push_arg(&mut fields, "pattern", arguments.pattern.clone());
    push_arg(&mut fields, "query", arguments.query.clone());
    push_arg(&mut fields, "input_csv", arguments.input_csv.clone());
    push_arg(
        &mut fields,
        "include_hidden",
        arguments.include_hidden.map(|value| value.to_string()),
    );
    push_arg(
        &mut fields,
        "offset",
        arguments.offset.as_ref().map(|value| value.to_string()),
    );
    push_arg(
        &mut fields,
        "limit",
        arguments.limit.as_ref().map(|value| value.to_string()),
    );
    push_arg(
        &mut fields,
        "max_results",
        arguments
            .max_results
            .as_ref()
            .map(|value| value.to_string()),
    );
    fields.sort();
    let canonical = fields.join("\n");
    format!(
        "weak:{tool_id}:{}",
        stable_text_hash(&canonical)
            .chars()
            .take(16)
            .collect::<String>()
    )
}

fn push_arg(fields: &mut Vec<String>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        fields.push(format!("{key}={value}"));
    }
}

fn bucket_file_read_max_bytes(value: usize) -> usize {
    let clamped = value.clamp(1, 80_000);
    clamped.next_power_of_two().min(80_000)
}

fn normalized_observation_path(arguments: &ParsedToolArguments) -> Option<String> {
    arguments
        .path
        .as_deref()
        .or(arguments.root.as_deref())
        .map(normalize_observation_segment)
}

fn normalized_observation_root(arguments: &ParsedToolArguments) -> Option<String> {
    arguments
        .root
        .as_deref()
        .or(arguments.path.as_deref())
        .map(normalize_observation_segment)
}

fn normalized_search_observation_target(
    arguments: &ParsedToolArguments,
) -> Option<(&'static str, String)> {
    if let Some(path) = arguments.path.as_deref() {
        return Some(("path", normalize_observation_segment(path)));
    }
    arguments
        .root
        .as_deref()
        .map(|root| ("root", normalize_observation_segment(root)))
}

fn normalize_observation_segment(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return ".".to_string();
    }
    let without_prefix = trimmed.strip_prefix("./").unwrap_or(trimmed);
    if without_prefix.is_empty() {
        ".".to_string()
    } else {
        without_prefix.trim_end_matches('/').to_string()
    }
}

fn plan_like_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("plan")
        || lower.contains("roadmap")
        || lower.contains("todo")
        || path.contains("计划")
}

fn file_read_mtime_ns(arguments: &ParsedToolArguments, workspace_root: &Path) -> Option<u128> {
    let raw_path = arguments.path.as_deref().or(arguments.root.as_deref())?;
    let root = workspace_root.canonicalize().ok()?;
    let raw = PathBuf::from(raw_path);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    };
    let resolved = candidate.canonicalize().ok()?;
    if !resolved.starts_with(&root) {
        return None;
    }
    let metadata = std::fs::metadata(resolved).ok()?;
    if !metadata.is_file() {
        return None;
    }
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_read_observation_is_detected_by_stable_key() {
        let args = ParsedToolArguments {
            path: Some("./README.md".to_string()),
            max_bytes: Some(4_000),
            ..ParsedToolArguments::default()
        };
        let mut cache = ObservationCache::default();
        assert_eq!(cache.check_and_record("file.read", &args), None);
        let duplicate = cache.check_and_record("file.read", &args).unwrap();
        assert_eq!(
            duplicate,
            "file.read:README.md:offset=0:limit=all:max_bytes=4096"
        );
        assert_eq!(cache.seen_count(&duplicate), 2);
    }

    #[test]
    fn file_read_key_buckets_max_bytes_perturbations() {
        let a = ParsedToolArguments {
            path: Some("src/lib.rs".to_string()),
            max_bytes: Some(8_000),
            ..ParsedToolArguments::default()
        };
        let b = ParsedToolArguments {
            path: Some("./src/lib.rs".to_string()),
            max_bytes: Some(8_192),
            ..ParsedToolArguments::default()
        };
        assert_eq!(
            observation_key("file.read", &a),
            observation_key("file.read", &b)
        );
    }

    #[test]
    fn unknown_read_only_tools_get_weak_dedup_hints_without_strong_contains() {
        let args = ParsedToolArguments {
            query: Some("symbol Foo".to_string()),
            root: Some("src".to_string()),
            ..ParsedToolArguments::default()
        };
        let mut cache = ObservationCache::default();
        assert_eq!(observation_key("repo.find_files", &args), None);
        assert!(!cache.contains("repo.find_files", &args));
        assert_eq!(
            cache.check_and_record_weak_hint("repo.find_files", &args),
            None
        );
        let hint = cache
            .check_and_record_weak_hint("repo.find_files", &args)
            .unwrap();
        assert!(hint.starts_with("weak:repo.find_files:"));
        assert!(!cache.contains("repo.find_files", &args));
    }

    #[test]
    fn search_key_prefers_path_and_preserves_root_compatibility() {
        let path_args = ParsedToolArguments {
            path: Some("./src/lib.rs".to_string()),
            root: Some("src".to_string()),
            pattern: Some("needle".to_string()),
            max_results: Some(25),
            ..ParsedToolArguments::default()
        };
        let root_args = ParsedToolArguments {
            root: Some("./src/lib.rs".to_string()),
            pattern: Some("needle".to_string()),
            max_results: Some(25),
            ..ParsedToolArguments::default()
        };
        assert_eq!(
            observation_key("search.ripgrep", &path_args),
            Some("search.ripgrep:path=src/lib.rs:pattern=needle:max_results=25".to_string())
        );
        assert_eq!(
            observation_key("search.ripgrep", &root_args),
            Some("search.ripgrep:root=src/lib.rs:pattern=needle:max_results=25".to_string())
        );
    }

    #[test]
    fn covered_read_range_is_detected_as_duplicate_observation() {
        let mut cache = ObservationCache::default();
        let broad = ParsedToolArguments {
            path: Some("plan/roadmap.md".to_string()),
            offset: Some(0),
            limit: Some(200),
            max_bytes: Some(16_000),
            ..ParsedToolArguments::default()
        };
        let narrow = ParsedToolArguments {
            path: Some("./plan/roadmap.md".to_string()),
            offset: Some(50),
            limit: Some(25),
            max_bytes: Some(8_000),
            ..ParsedToolArguments::default()
        };
        assert_eq!(cache.check_and_record("file.read", &broad), None);
        let duplicate = cache.check_and_record("file.read", &narrow).unwrap();
        assert!(duplicate.contains("covered_by=file.read:plan/roadmap.md"));
        assert!(cache.contains("file.read", &narrow));
    }

    #[test]
    fn adjacent_read_range_is_allowed() {
        let mut cache = ObservationCache::default();
        let first = ParsedToolArguments {
            path: Some("plan/roadmap.md".to_string()),
            offset: Some(0),
            limit: Some(100),
            max_bytes: Some(16_000),
            ..ParsedToolArguments::default()
        };
        let next = ParsedToolArguments {
            path: Some("plan/roadmap.md".to_string()),
            offset: Some(100),
            limit: Some(100),
            max_bytes: Some(16_000),
            ..ParsedToolArguments::default()
        };
        assert_eq!(cache.check_and_record("file.read", &first), None);
        assert_eq!(cache.check_and_record("file.read", &next), None);
    }

    #[test]
    fn larger_max_bytes_read_is_not_covered_by_smaller_read() {
        let mut cache = ObservationCache::default();
        let small = ParsedToolArguments {
            path: Some("plan/roadmap.md".to_string()),
            offset: Some(0),
            limit: None,
            max_bytes: Some(8_000),
            ..ParsedToolArguments::default()
        };
        let larger = ParsedToolArguments {
            path: Some("plan/roadmap.md".to_string()),
            offset: Some(50),
            limit: Some(20),
            max_bytes: Some(16_000),
            ..ParsedToolArguments::default()
        };
        assert_eq!(cache.check_and_record("file.read", &small), None);
        assert_eq!(cache.check_and_record("file.read", &larger), None);
    }

    #[test]
    fn repeated_plan_reads_hit_exploration_budget() {
        let mut cache = ObservationCache::default();
        for index in 0..4 {
            let args = ParsedToolArguments {
                path: Some("plan/VoiceNote-AI-实施计划.md".to_string()),
                offset: Some(index * 100),
                limit: Some(50),
                max_bytes: Some(8_000 + index),
                ..ParsedToolArguments::default()
            };
            assert_eq!(cache.check_and_record("file.read", &args), None);
        }
        let extra = ParsedToolArguments {
            path: Some("plan/VoiceNote-AI-实施计划.md".to_string()),
            offset: Some(500),
            limit: Some(50),
            max_bytes: Some(16_000),
            ..ParsedToolArguments::default()
        };
        let duplicate = cache.check_and_record("file.read", &extra).unwrap();
        // Stable key — no attempts=N salt
        assert!(duplicate.contains("file.read:plan/VoiceNote-AI-实施计划.md"));
        assert!(!duplicate.contains("plan_read_budget_exhausted"));
        // Verify via outcome API too
        let mut cache2 = ObservationCache::default();
        for index in 0..4 {
            let args = ParsedToolArguments {
                path: Some("plan/VoiceNote-AI-实施计划.md".to_string()),
                offset: Some(index * 100),
                limit: Some(50),
                max_bytes: Some(8_000 + index),
                ..ParsedToolArguments::default()
            };
            assert!(cache2
                .check_and_record_with_outcome("file.read", &args)
                .is_none());
        }
        let outcome = cache2
            .check_and_record_with_outcome("file.read", &extra)
            .unwrap();
        assert!(matches!(
            outcome,
            DedupeOutcome::DuplicateRateLimited { .. }
        ));
    }

    #[test]
    fn file_read_dedup_respects_mtime_change() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("researchcode-observation-cache-mtime-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("README.md");
        std::fs::write(&path, "first\n").unwrap();
        let args = ParsedToolArguments {
            path: Some("README.md".to_string()),
            max_bytes: Some(4_000),
            ..ParsedToolArguments::default()
        };
        let mut cache = ObservationCache::default();
        assert_eq!(
            cache.check_and_record_in_workspace("file.read", &args, &root),
            None
        );
        assert!(cache
            .check_and_record_in_workspace("file.read", &args, &root)
            .is_some());

        let mut invalidated = false;
        for index in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            std::fs::write(&path, format!("second {index}\n")).unwrap();
            if cache
                .check_and_record_in_workspace("file.read", &args, &root)
                .is_none()
            {
                invalidated = true;
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&root);
        assert!(invalidated, "mtime change should allow a fresh file.read");
    }
}
