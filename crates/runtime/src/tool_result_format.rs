//! Doc39 §14 tool result formatting primitives.
//!
//! Each formatter produces a preview string in the format DeepSeek/Qwen models
//! prefer: line-numbered file reads, base→new hash diffs for edits, command
//! lines with exit code + duration + stdout tail + stderr for shell.
//!
//! Wiring into the live execution paths is intentionally deferred — these
//! formatters are pure functions over primitive arguments, callable from
//! `tool_execution.rs` once we are ready to switch the preview shape.

/// Format the preview emitted by `file.read`.
///
/// Inputs are deliberately primitive so the formatter can be used both inline
/// (live execution path) and offline (replay / fixture generation):
///
/// * `path` — relative or absolute path string as already redacted upstream
/// * `content` — exactly the slice the user requested (already truncated to the
///   physical line window)
/// * `offset` — 1-based first line included in `content`
/// * `limit` — number of lines actually returned
/// * `total_lines` — total physical lines in the file (post-redaction)
pub fn format_file_read_preview(
    path: &str,
    content: &str,
    offset: usize,
    limit: usize,
    total_lines: usize,
) -> String {
    let first_line = offset.max(1);
    let end_line = first_line
        .saturating_add(limit.saturating_sub(1))
        .min(total_lines.max(1));
    let header = format!("file.read · {path} · lines {first_line}-{end_line}/{total_lines}");

    let mut numbered = String::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = first_line + idx;
        numbered.push_str(&format!("{line_no:>4}  {line}\n"));
    }
    if numbered.is_empty() {
        numbered.push_str("(no lines in window)\n");
    }

    let remaining = total_lines.saturating_sub(end_line);
    let truncated_tail = if remaining > 0 {
        let next_offset = end_line + 1;
        format!(
            "\n[truncated; {remaining} more lines, request offset={next_offset},limit=N to continue]"
        )
    } else {
        String::new()
    };

    format!("{header}\n\n{numbered}{truncated_tail}")
}

/// Format the preview emitted by `file.edit` on success.
///
/// `base_hash` and `new_hash` should already be in their stable display format.
/// `before`/`after` are the full file contents before/after the edit; we emit a
/// minimal unified-diff fragment (line markers `-` / `+` with 3 lines of
/// context around each hunk) so the model can reason about exactly what
/// changed without paying for a large diff library.
pub fn format_file_edit_preview(
    path: &str,
    replacements: usize,
    base_hash: &str,
    new_hash: &str,
    before: &str,
    after: &str,
) -> String {
    let header = format!("file.edit · {path} · {replacements} replacement{plural}\n\nbase_hash: {base_hash} → new_hash: {new_hash}",
        plural = if replacements == 1 { "" } else { "s" });
    let diff = simple_unified_diff(before, after);
    if diff.is_empty() {
        header
    } else {
        format!("{header}\n\n{diff}")
    }
}

/// Format the preview emitted by `shell.command`.
///
/// `stdout`/`stderr` are expected to already be secret-redacted upstream.
/// We tail the stdout to the most recent 80 lines and always emit a `stderr:`
/// section even when empty so the model never has to guess.
pub fn format_shell_command_preview(
    command: &str,
    exit_code: i32,
    duration_ms: u128,
    stdout: &str,
    stderr: &str,
) -> String {
    let header = format!("shell.command · `{command}` · exit {exit_code} · {duration_ms}ms");

    let stdout_tail = tail_lines(stdout, 80);
    let stdout_block = if stdout_tail.is_empty() {
        "stdout (last 80 lines):\n(empty)".to_string()
    } else {
        format!("stdout (last 80 lines):\n{stdout_tail}")
    };
    let stderr_block = if stderr.trim().is_empty() {
        "stderr: (empty)".to_string()
    } else {
        format!("stderr:\n{}", stderr.trim_end())
    };
    format!("{header}\n\n{stdout_block}\n\n{stderr_block}")
}

pub fn format_file_write_preview(
    path: &str,
    bytes_written: usize,
    content_hash: &str,
    rollback_artifact: &str,
) -> String {
    format!(
        "file.write · {path} · wrote {bytes_written} bytes\n\ncontent_hash: {content_hash}\nrollback: {rollback_artifact}"
    )
}

pub fn format_file_multi_edit_preview(
    path: &str,
    replacement_count: usize,
    base_hash: &str,
    new_hash: &str,
    rollback_artifact: &str,
) -> String {
    format!(
        "file.multi_edit · {path} · applied {replacement_count} replacement{plural}\n\nbase_hash: {base_hash} → new_hash: {new_hash}\nrollback: {rollback_artifact}",
        plural = if replacement_count == 1 { "" } else { "s" }
    )
}

pub fn format_list_directory_preview(
    path: &str,
    entry_count: usize,
    omitted_count: usize,
) -> String {
    format!("file.list_directory · {path} · listed {entry_count} entries · omitted={omitted_count}")
}

pub fn format_list_tree_preview(
    path: &str,
    line_count: usize,
    file_count: usize,
    omitted_count: usize,
) -> String {
    format!(
        "file.list_tree · {path} · tree lines={line_count} files={file_count} omitted={omitted_count}"
    )
}

fn tail_lines(text: &str, n: usize) -> String {
    if text.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Minimal unified-diff fragment generator. Not byte-perfect with `diff -u`,
/// but emits hunks with `-` / `+` line markers + up to 3 surrounding context
/// lines per change. Deterministic; no external dependencies.
fn simple_unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();

    // Compute LCS table (line-level). Keep this O(N*M) — file.edit operates on
    // bounded windows so this is fine in practice.
    let n = a.len();
    let m = b.len();
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            lcs[i + 1][j + 1] = if a[i] == b[j] {
                lcs[i][j] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    // Walk back to emit ops in (kind, line) form.
    #[derive(Debug, Clone)]
    enum Op<'a> {
        Same(&'a str),
        Del(&'a str),
        Add(&'a str),
    }
    let mut ops: Vec<Op> = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            ops.push(Op::Same(a[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || lcs[i][j - 1] >= lcs[i - 1][j]) {
            ops.push(Op::Add(b[j - 1]));
            j -= 1;
        } else {
            ops.push(Op::Del(a[i - 1]));
            i -= 1;
        }
    }
    ops.reverse();

    // Group into hunks with up to 3 lines of leading/trailing context.
    const CTX: usize = 3;
    let mut out = String::new();
    let mut idx = 0;
    while idx < ops.len() {
        if matches!(ops[idx], Op::Same(_)) {
            idx += 1;
            continue;
        }
        let hunk_start = idx.saturating_sub(CTX);
        let mut hunk_end = idx;
        while hunk_end < ops.len() {
            if matches!(ops[hunk_end], Op::Same(_)) {
                let trailing_same = (hunk_end..ops.len())
                    .take_while(|k| matches!(ops[*k], Op::Same(_)))
                    .count();
                if trailing_same > CTX * 2 {
                    hunk_end += CTX;
                    break;
                }
                hunk_end += trailing_same;
            } else {
                hunk_end += 1;
            }
        }
        if hunk_end > ops.len() {
            hunk_end = ops.len();
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("@@\n");
        for op in &ops[hunk_start..hunk_end] {
            match op {
                Op::Same(line) => out.push_str(&format!("  {line}\n")),
                Op::Del(line) => out.push_str(&format!("- {line}\n")),
                Op::Add(line) => out.push_str(&format!("+ {line}\n")),
            }
        }
        idx = hunk_end;
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_read_preview_emits_line_numbers_and_truncation_note() {
        let preview = format_file_read_preview(
            "src/foo.rs",
            "pub fn main() {\n    println!(\"hi\");\n}",
            1,
            3,
            200,
        );
        assert!(preview.starts_with("file.read · src/foo.rs · lines 1-3/200"));
        assert!(preview.contains("   1  pub fn main() {"));
        assert!(preview.contains("   2      println!(\"hi\");"));
        assert!(preview.contains("   3  }"));
        assert!(
            preview.contains("[truncated; 197 more lines, request offset=4,limit=N to continue]")
        );
    }

    #[test]
    fn file_read_preview_handles_eof_window() {
        let preview = format_file_read_preview("README.md", "last line", 10, 1, 10);
        assert!(preview.contains("lines 10-10/10"));
        assert!(!preview.contains("[truncated"));
    }

    #[test]
    fn file_edit_preview_shows_hash_arrow_and_diff() {
        let preview = format_file_edit_preview(
            "src/foo.rs",
            1,
            "7a3b",
            "9c4d",
            "fn main() {\n    println!(\"hi\");\n}",
            "fn run() {\n    println!(\"hi\");\n}",
        );
        assert!(preview.starts_with("file.edit · src/foo.rs · 1 replacement"));
        assert!(preview.contains("base_hash: 7a3b → new_hash: 9c4d"));
        assert!(preview.contains("- fn main() {"));
        assert!(preview.contains("+ fn run() {"));
    }

    #[test]
    fn file_edit_preview_uses_plural_replacements_label() {
        let preview = format_file_edit_preview("src/foo.rs", 3, "a", "b", "x\n", "y\n");
        assert!(preview.starts_with("file.edit · src/foo.rs · 3 replacements"));
    }

    #[test]
    fn shell_command_preview_emits_stderr_block_when_empty() {
        let preview = format_shell_command_preview(
            "cargo test --lib",
            0,
            3412,
            "running 314 tests\ntest ... ok",
            "",
        );
        assert!(preview.starts_with("shell.command · `cargo test --lib` · exit 0 · 3412ms"));
        assert!(preview.contains("stdout (last 80 lines):"));
        assert!(preview.contains("running 314 tests"));
        assert!(preview.contains("stderr: (empty)"));
    }

    #[test]
    fn shell_command_preview_tails_stdout_to_80_lines() {
        let stdout = (1..=200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let preview = format_shell_command_preview("seq 200", 0, 12, &stdout, "");
        assert!(preview.contains("line 200"));
        assert!(preview.contains("line 121"));
        assert!(!preview.contains("line 120"));
    }

    #[test]
    fn shell_command_preview_keeps_stderr_when_present() {
        let preview = format_shell_command_preview("bad", 1, 1, "", "boom\n");
        assert!(preview.contains("stderr:\nboom"));
    }

    #[test]
    fn shell_command_preview_marks_empty_stdout() {
        let preview = format_shell_command_preview("true", 0, 1, "", "");
        assert!(preview.contains("stdout (last 80 lines):\n(empty)"));
    }

    #[test]
    fn file_write_preview_includes_hash_and_rollback() {
        let preview = format_file_write_preview("src/lib.rs", 128, "abc123", ".rollback/r1");
        assert!(preview.contains("file.write · src/lib.rs · wrote 128 bytes"));
        assert!(preview.contains("content_hash: abc123"));
        assert!(preview.contains("rollback: .rollback/r1"));
    }

    #[test]
    fn file_multi_edit_preview_uses_plural_and_hash_arrow() {
        let preview = format_file_multi_edit_preview("src/lib.rs", 2, "h1", "h2", ".rollback/r2");
        assert!(preview.contains("file.multi_edit · src/lib.rs · applied 2 replacements"));
        assert!(preview.contains("base_hash: h1 → new_hash: h2"));
    }

    #[test]
    fn list_directory_preview_has_counts() {
        let preview = format_list_directory_preview("src", 10, 3);
        assert!(preview.contains("listed 10 entries"));
        assert!(preview.contains("omitted=3"));
    }

    #[test]
    fn list_tree_preview_has_shape_metrics() {
        let preview = format_list_tree_preview(".", 20, 8, 1);
        assert!(preview.contains("tree lines=20"));
        assert!(preview.contains("files=8"));
        assert!(preview.contains("omitted=1"));
    }
}
