# Red Team Report: Security & Correctness

## Methodology

Cross-referenced all Phase 1 audit reports and the Issue Matrix against live source code at:

- `crates/runtime/src/agent_kernel/permission_gate.rs` (full file, 1168 lines)
- `crates/runtime/src/runtime_facade_impl.rs` (Sec. 1-600, 3040-3210, 5290-5470)
- `crates/runtime/src/command.rs` (full file, 317 lines)
- `crates/runtime/src/runtime/permission_service.rs` (full file, 253 lines)
- `crates/runtime/src/tool_execution.rs` (Sec. 141-428, 487-536, 895-950, 1985-2078)
- `crates/runtime/src/file_tool.rs` (Sec. 95-112)
- `crates/kernel/src/tool.rs` (Sec. 28-47, 98-163, 600)

Traced every shell command path from model output through classifier to execution, both native loop and facade entry points.

---

## Part 1: Missed Security Vulnerabilities

### Finding RED-01: Shell interpreters (`sh`, `bash`, `zsh`) not blocked at any layer

**Severity: P0**

**Files and lines:**
- `permission_gate.rs:191-198` — program deny list: `["rm", "mv", "cp", "chmod", "chown", "kill", "pkill"]` — does NOT include `sh`, `bash`, `zsh`, `dash`
- `permission_gate.rs:35-57` — DENY_SUBSTRINGS — does NOT include `sh -c`, `bash -c`
- `runtime_facade_impl.rs:3192-3209` — `command_contains_hard_deny` — does NOT include `sh`, `bash`
- `command.rs:124-148` — `run_prepared_command` — `authorize_command` only checks `plan.blocked`, and `plan.blocked` is `true` only when the classifier returns `CommandDecision::Deny`

**What we missed:**
Shell interpreters pass through all three defense layers unblocked. `sh -c "echo hello"`:
1. Layer A (`command_contains_hard_deny`): no match → passes
2. Layer B (`classify_command_with_reasons`): program `sh` not in any deny list, not in allowlist → `CommandDecision::Ask` → `PermissionResolution::Ask` (user-approvable)
3. Layer C (`run_prepared_command`): `plan.blocked = false` (classifier returned Ask, not Deny) → `authorize_command` returns `RequiresPermission` → with user approval, executes `Command::new("sh").args(["-c", "echo hello"])`

**Why it matters:**
While `sh -c "rm -rf /"` would be caught by DENY_SUBSTRINGS ("rm -rf"), more subtle payloads pass through:
- `sh -c "python3 -c 'import base64; exec(base64.b64decode(\"...\"))'"` — no dangerous substring, program `sh` not denied → Ask → user-approvable
- `sh -c "$(cat /etc/passwd > /tmp/exfil)"` — wait, this contains `$(` which IS caught by shell operator check at permission_gate.rs:163 → Deny → blocked at execution

The primary risk is encoded/obfuscated payloads through `sh -c` that contain no substring matches against DENY_SUBSTRINGS. The model could be instructed (or hallucinate) multi-step attacks where the dangerous substring appears only after decoding at runtime.

**Suggested fix:**
Add `"sh"`, `"bash"`, `"zsh"`, `"dash"` to the program deny list at `permission_gate.rs:191-198`. Shell interpreters should never be directly invoked; the system already handles command execution via `std::process::Command` without a shell wrapper.

---

### Finding RED-02: Unicode/tab whitespace bypass for DENY_SUBSTRINGS substring matching

**Severity: P1**

**Files and lines:**
- `permission_gate.rs:159` — `if DENY_SUBSTRINGS.iter().any(|part| lowered.contains(part))` — substring check on raw command string using ASCII space delimiters
- `permission_gate.rs:329` — `if char.is_whitespace()` — tokenizer uses Unicode-aware whitespace

**What we missed:**
The DENY_SUBSTRINGS list contains patterns with ASCII spaces: `"sudo "`, `" rm "`, `" -exec "`, `" -ok "`. The substring check at line 159 compares against `command.to_lowercase()`, which preserves non-ASCII whitespace characters unchanged. But the tokenizer at line 329 uses `char.is_whitespace()` which matches tabs (U+0009), no-break spaces (U+00A0), and other Unicode whitespace.

Commands with tabs instead of spaces can bypass substring matching while still being correctly tokenized:

```text
Input:           "sudo\tfind /etc"
lowered:          "sudo\tfind /etc"
"sudo " match?    NO (tab, not space)
" rm " match?     N/A
substring DENY:   none matched → passes substring phase

Tokenization:     ["sudo", "find", "/etc"]  (tab splits correctly)
Program check:    "sudo" not in program deny list → falls through to Ask
```

**Why it matters:**
The model could learn to emit commands with tabs (common in shell scripts), bypassing the substring-based DENY checks for `sudo`, `rm` (space-delimited), and `find`'s `-exec`/`-ok` flags. While the `rm` bypass is limited because `rm` IS in the program deny list (token-based), `sudo` is checked ONLY via the DENY_SUBSTRINGS `"sudo "` pattern and NOT in the program deny list. So `sudo\t<anything>` fully bypasses the classifier.

**Suggested fix:**
Two independent changes:
1. In DENY_SUBSTRINGS matching: replace ASCII spaces with a whitespace-agnostic check, or normalize all Unicode whitespace to ASCII space before substring matching.
2. Add `"sudo"` to the program deny list at line 191-198 as defense-in-depth, independent of substring whitespace handling.

---

### Finding RED-03: Native agent loop execution path skips Layer A hard deny entirely

**Severity: P1**

**Files and lines:**
- `native_agent_loop_execution.rs:447` — `execute_tool_with_permission_gate(...)` — uses PermissionGate (Layer B only)
- `runtime_facade_impl.rs:3040-3062` — `facade_tool_mode` — applies Layer A (`command_contains_hard_deny`) before Layer B
- `tool_execution.rs:154-190` — `execute_tool_inner` — dispatches to `execute_shell_command` with gate-resolved permission, no Layer A equivalent

**What we missed:**
The Phase 1 audit describes a 3-layer defense but only audited the facade path. There are two distinct entry points:

| Entry point | Layer A (hard deny) | Layer B (classifier) | Layer C (execution block) |
|---|---|---|---|
| Facade (`facade_tool_mode`) | YES | YES (via `apply_permission_policy`) | YES |
| Native loop (`execute_tool_with_permission_gate`) | **NO** | YES (via `PermissionGate`) | YES |

`command_contains_hard_deny` is called at `runtime_facade_impl.rs:3054` within `facade_tool_mode`, which is used by Tauri/web endpoints. The native agent loop at `native_agent_loop_execution.rs:447` calls `execute_tool_with_permission_gate` directly, which only uses `PermissionGate::evaluate_current` → `ShellCommandTool::check_permissions` → `classify_command_with_reasons` (Layer B only).

**Why it matters:**
Commands blocked immediately by Layer A in the facade path become user-approvable in the native loop path:
- `npm install` — Layer A hard denies; native loop asks user (classified as AskPackageInstall → Ask)
- `|` — Layer A hard denies; native loop classifier catches as shell operator → Deny → blocked at Layer C execution anyway
- `$(...)` — Layer A hard denies; native loop classifier catches → Deny → blocked at Layer C anyway

For `npm install` / `pip install` specifically, the difference is real: the facade blocks immediately, the native loop allows with user approval. An attacker controlling the prompt entry point could choose the native loop path to bypass Layer A.

**Suggested fix:**
Inject a `command_contains_hard_deny` equivalent into the `execute_tool_inner` path for `shell.command`, OR ensure all entry points route through a single `facade_tool_mode` check before tool execution. The latter is preferred: one function, one source of truth.

---

### Finding RED-04: `is_sensitive_path` coverage gap vs `check_dangerous_path`

**Severity: P2**

**Files and lines:**
- `file_tool.rs:95-112` — `is_sensitive_path` — checks only: `.env`, `id_rsa`, `.ssh`, `private_key`, `.pem`, `.key`, `.pfx`, `credentials`, `token`, `/proc/`, `/dev/fd/`
- `permission_gate.rs:599-663` — `check_dangerous_path` — checks: `..`, DOS devices, NTFS streams, 8.3 names, UNC paths, dangerous dirs (`.git`, `.vscode`, `.idea`, `.claude`, `.ssh`), dangerous files (`.env`, `.gitconfig`, `.bashrc`, `.zshrc`, `id_rsa`, `id_ed25519`, `credentials`, `.npmrc`, `secrets.json`, etc.)

**What we missed:**
`execute_file_read_preview` calls `resolve_within_workspace` which canonicalizes symlinks THEN calls `is_sensitive_path`. This is symlink-safe but `is_sensitive_path` checks far fewer dangerous patterns than `check_dangerous_path`. Specifically, `is_sensitive_path` does NOT check for:
- Dangerous directories: `.git`, `.vscode`, `.idea`, `.claude`
- Dangerous config files: `.gitconfig`, `.bashrc`, `.zshrc`, `.profile`, `.npmrc`, `.pypirc`, `.netrc`
- Secrets files: `secrets.json`, `secrets.yml`, `secrets.yaml`, `token.txt`
- DOS device names, NTFS streams, 8.3 names

**Why it matters:**
A model can read `.git/config` or `.npmrc` (which may contain auth tokens) without triggering any sensitive path check. The permission gate for FileReadTool returns `Passthrough` (inherited default), so there's no pre-execution gate either. The only defense is the user's permission prompt, which doesn't highlight the sensitivity of these files.

**Suggested fix:**
Unify `is_sensitive_path` with `check_dangerous_path`, or at minimum add the dangerous directories and config/secret file checks from `check_dangerous_path` into `is_sensitive_path`.

---

### Finding RED-05: No permission submission rate limiting

**Severity: P2**

**Files and lines:**
- `runtime_facade_impl.rs:365-597` — `resume_native_loop_after_permission_decision` — no rate limit, no flood detection

**What we missed:**
A malicious or buggy UI client can submit thousands of permission decisions per second for the same session. While `permission_id` matching prevents double-execution of the same pending decision, the lock/unlock cycle on the session mutex creates a DoS vector through lock contention.

**Suggested fix:**
Add a simple cooldown (e.g., 100ms minimum between permission submissions for the same session) or a submission counter with a hard cap per session.

---

### Finding RED-06: `artifact.export` has no security classifier

**Severity: P2**

**Files and lines:**
- `runtime_facade_impl.rs:3074-3076` — gated only by `RequirePermission(PermissionRequestType::ArtifactExport)`, no content check
- `permission_gate.rs:1164` — ArtifactExport maps to `DefaultTool` with `is_state_changing: false`, `is_read_only: false` — no `check_permissions` override
- `tool_execution.rs:224` — ArtifactExport can dispatch through `ApplyWithPermission` path

**What we missed:**
Artifact export has no classifier, no path safety check, and no secret scanning. A model could export session artifacts containing sensitive data (tool outputs with secrets, file contents from the workspace) without any automated security inspection. The only gate is the user permission prompt.

**Suggested fix:**
Run `redact_text_for_secrets` on exported artifact content (similar to what `capture_command_output_artifact` does in `command.rs:106-107`). Add a path/content classifier for exported files.

---

## Part 2: Severity Challenges

### Challenge RED-S1: P2-17 (sudo not in Layer A) should be P1

The audit rates "sudo not in Layer A hard block" as P2. This understates the risk because:

1. `sudo` is checked ONLY via DENY_SUBSTRINGS (`"sudo "` with ASCII space) — it is NOT in the token-based program deny list
2. Combined with Finding RED-02 (Unicode whitespace bypass), `sudo\t<anything>` bypasses ALL substring and program checks
3. Combined with Finding RED-01 (shell interpreters not blocked), `sudo sh -c` gives the model root shell access with only user approval
4. The only defense against `sudo rm -rf /` is the DENY_SUBSTRINGS catch of `"rm -rf"` in the raw command string — if the model obfuscates the payload, this fails

**Recommendation:** Upgrade P2-17 to P1. Add `"sudo"` to the token-based program deny list (not just substring matching).

---

### Challenge RED-S2: P2-19 (missing dangerous programs) should be P1

The audit rates missing programs (`mkfs`, `dd`, `fdisk`, `systemctl`, `mount`, `umount`, `shutdown`, `reboot`) as P2. While individually many of these require root, they represent irreversible destructive operations:

- `mkfs.ext4 /dev/sda` — formats a disk partition with no confirmation
- `dd if=/dev/zero of=/dev/sda` — overwrites raw disk
- `fdisk /dev/sda` — modifies partition table
- `systemctl stop critical-service` — stops system services
- `shutdown -h now` — shuts down the system

Even without root, `systemctl --user stop` can disrupt user services. The worst-case impact (irreversible data loss) warrants P1.

**Recommendation:** Upgrade P2-19 to P1. Extend the program deny list to include all disk/process management utilities.

---

### Challenge RED-S3: P1-17 (classifier Deny not true deny) should stay P1 but description needs correction

The audit states: "Classifier `Deny` becomes `ToolPermissionResult::SafetyCheck { classifier_approvable: false }` → `PermissionResolution::Ask { safety: true }` → user CAN approve." This is technically correct about the permission flow, but misleading about the security outcome.

**What the audit missed:**
`command.rs:run_prepared_command` provides an execution-layer backstop:

```rust
// command.rs:56-63
let blocked = classifier_decision == CommandDecision::Deny;

// command.rs:85-87
if plan.blocked {
    return CommandAuthorization::BlockedByPolicy;
}
```

Even when the permission gate returns `Ask` for a classifier-Denied command, and even if the user approves, `run_prepared_command` will refuse to execute it. The user gets a confusing UX (approve → still blocked) but the command does NOT execute.

**The real risk is UX confusion, not security bypass.** A user who approves a `SafetyCheck` prompt and sees it fail with an opaque error might be conditioned to ignore future warnings, degrading the effectiveness of the permission system.

**Recommendation:** Keep P1-17 at P1 but clarify the finding: "Classifier Deny is presented as user-approvable in the permission UI, creating misleading UX. While the execution layer provides a backstop that prevents actual execution, the inconsistency between permission-layer verdict (Ask) and execution-layer verdict (BlockedByPolicy) erodes trust in the permission system and may condition users to approve future warnings."

---

### Challenge RED-S4: P2-16 (rmdir not blocked) — correctly rated P2

`rmdir` only removes empty directories. While it could be used for cleanup after a malicious operation, it cannot cause direct data destruction. P2 is appropriate.

---

## Part 3: False Positives / Overstated Findings

### False Positive RED-F1: "FileRead path has no path traversal protection"

**Audit claim (implicit from P0 positive finding #10):** "Path traversal protection comprehensive for file tools" — correct.

**Investigation:** The audit's positive finding is actually correct, but I initially suspected a gap. File reads go through `execute_file_read_preview` → `resolve_within_workspace` which:
1. Canonicalizes the workspace root (`tool_execution.rs:1992-1994`)
2. Canonicalizes the resolved path (`tool_execution.rs:1996-1998`)
3. Calls `is_sensitive_path` on the CANONICALIZED path (`tool_execution.rs:2000-2002`)
4. Checks the canonicalized path stays within workspace (`tool_execution.rs:2003-2005`)

Symlinks are fully resolved before the sensitive path check, so symlinks to `.ssh/id_rsa` inside the workspace WOULD be caught by `is_sensitive_path`. Symlinks to outside the workspace are caught by the boundary check. **No false positive — the audit is correct.**

However, `is_sensitive_path` covers fewer patterns than `check_dangerous_path` (see RED-04), which is a coverage gap, not an architectural flaw.

---

### False Positive RED-F2: "Symlink attack on file writes possible"

**My initial concern:** `check_dangerous_path` operates on raw (non-canonicalized) paths, so a symlink could hide the true target.

**Investigation:** `resolve_write_path_within_workspace` (`tool_execution.rs:2009-2078`) provides defense-in-depth:
1. Parent directory is canonicalized (`tool_execution.rs:2023-2026`)
2. Canonicalized parent checked against workspace root (`tool_execution.rs:2047-2049`)
3. `..` components are normalized before final check (`tool_execution.rs:2055-2068`)
4. **Symlink final path components are explicitly rejected** (`tool_execution.rs:2075-2078`)

A symlink in the final path component (the file being written) causes the operation to fail. A symlink in an intermediate directory is caught because the parent canonicalization would resolve it, and the resolved path must be within the workspace.

**Verdict: No bypass exists. The audit's positive finding #10 is accurate.**

---

### False Positive RED-F3: "PermissionService creates new gate per evaluation" — doc39 violation is real, security impact is overstated

**Audit finding (P2-18):** "PermissionService creates new gate per evaluation — breaks long-lived gate design"

**Investigation:** The `DenialTracker` (consecutive denials, total denials) is reset per evaluation when `apply_permission_policy` creates a new gate. However:
- The `PermissionRuleStore` is shared (same `Arc`) across all gates — persistent user rules are preserved
- The `DenialTracker.should_fallback()` mechanism (trigger at 3 consecutive / 20 total denials) never activates in the `apply_permission_policy` path because the counter always starts at zero
- The native loop path uses a persistent gate (created at loop start), so `should_fallback()` works correctly there

**Verdict:** The audit finding is factually correct and the doc39 violation is real. However, the security impact is limited because persistent rules still work. The main loss is the denial-based fallback detection in the facade path. P2 is the correct rating. **Not overstated.**

---

## Part 4: Un-audited Execution Paths

### Un-audited Path UA-01: Dual permission architecture (facade vs. native loop)

**Not covered by any Phase 1 agent.** The facade path and native agent loop path implement permission evaluation through different code paths with different security properties (see RED-03). No audit agent traced BOTH paths and compared their security guarantees.

**Impact:** Commands blocked at Layer A in the facade path are user-approvable in the native loop path. The security model depends on which entry point is used, and this is not documented or tested.

**Files involved:**
- Facade: `runtime_facade_impl.rs:3040-3062` → `permission_service.rs:55-100` → `permission_gate.rs:696-945`
- Native loop: `native_agent_loop_execution.rs:447` → `tool_execution.rs:147-190` → `permission_gate.rs:696-945`

**Recommendation:** Audit as a cross-cutting concern. Either unify the paths or document the gap explicitly.

---

### Un-audited Path UA-02: Plan approval has no automated safety checks

**Not covered by any Phase 1 agent.** The plan approval path (`runtime_facade_impl.rs:599-684`, `submit_plan_decision`) has no classifier, no path check, no hard deny equivalent. A plan is model-generated text that describes a workflow. Plans that include dangerous operations (e.g., "Step 1: run `sudo rm -rf /`") have no automated safety gate — only the plan-approval user prompt.

**Impact:** A model could generate a plan containing dangerous commands, and the plan approval UI would present it without any security warnings. The user approves the PLAN, not individual tool executions, so the security implication is that dangerous operations could be approved in bulk through plan approval.

**Recommendation:** Run plan text through a subset of the classifier (at minimum DENY_SUBSTRINGS) and flag / warn on matches.

---

### Un-audited Path UA-03: `artifact.export` content inspection

**Not covered by any Phase 1 agent.** `artifact.export` (`runtime_facade_impl.rs:3074-3076`) gates on user permission only. The exported content is never scanned for secrets or sensitive data, unlike command output which goes through `redact_text_for_secrets` in `capture_command_output_artifact` (`command.rs:106-107`).

**Impact:** A model could export session artifacts containing tool output with secrets (API keys, tokens, credentials from file reads) without any automated redaction.

**Recommendation:** Apply `redact_text_for_secrets` to exported artifact content, matching the protection already present for command output.

---

### Un-audited Path UA-04: Concurrent tool execution path (`execute_tool_batch_concurrent`)

**Not covered by any Phase 1 agent.** The concurrent batch execution path at `tool_execution.rs:290-340+` executes tools in parallel with a sibling abort controller. This path has its own permission resolution: `execute_tool_batch_concurrent` receives individual tool requests, each going through `execute_tool_inner` independently. There is no cross-tool permission analysis — two individually safe tools could combine into a dangerous operation when executed concurrently (e.g., `file.read` on a lock file while `file.write` modifies the locked resource).

**Impact:** Race conditions between concurrent tool executions could produce correctness bugs or data corruption. The TCML bypass for read-only concurrent tools (audit finding P1-09) is related but focuses on the TCML mediation gap, not the concurrent execution safety.

**Recommendation:** Add cross-tool safety analysis for concurrent batches, or require sequential execution for tools that operate on the same file paths.

---

### Un-audited Path UA-05: `FileReadTool::check_permissions` always returns Passthrough

**Not covered by any Phase 1 agent.** `FileReadTool` at `permission_gate.rs:439-457` does NOT implement `check_permissions`. The default trait implementation returns `ToolPermissionResult::Passthrough`. This means:

1. The permission gate evaluates `file.read` requests and ALWAYS returns the default mode-based decision (Allow in most modes) — no path safety check
2. The actual safety check happens later at execution time in `resolve_within_workspace` → `is_sensitive_path`
3. But the gate never flags a read as dangerous, so the user never sees a "this file is sensitive" warning for reads

**Impact:** The user can see a "file.read" permission prompt without any indication that the file is sensitive. The execution layer would block the actual read, but the permission prompt is misleading. Compare with writes, where `FileWriteTool::check_permissions` calls `check_dangerous_path` and can return `SafetyCheck`.

**Recommendation:** Implement `check_permissions` on `FileReadTool` to call `check_dangerous_path` (or at minimum `is_sensitive_path`), so the permission prompt accurately reflects file sensitivity.

---

## Part 5: Cross-Reference with Existing Issue Matrix

| Red Team Finding | Relates to | Relationship |
|---|---|---|
| RED-01 (shell interpreters not blocked) | NEW | Independent finding |
| RED-02 (Unicode whitespace bypass) | P2-17 (sudo not in Layer A) | Amplifies P2-17: the bypass makes the sudo gap exploitable |
| RED-03 (native loop skips Layer A) | P2-07 (prompt keywords drive exposure) | Same architecture: native loop has different security properties than facade |
| RED-04 (is_sensitive_path coverage gap) | P2-18 (path traversal weaker for shell) | Same class: incomplete coverage analysis |
| RED-05 (no permission rate limiting) | P1-04 (unlocked window race) | Amplifies: flooding makes race windows easier to hit |
| RED-06 (artifact export no classifier) | NEW | Independent finding |
| RED-S1 (P2-17 → P1) | P2-17 | Severity upgrade |
| RED-S2 (P2-19 → P1) | P2-19 | Severity upgrade |
| RED-S3 (P1-17 clarification) | P1-17 | Finding correction, not severity change |
| RED-F1, RED-F2 | Positive finding #10 | Confirmed correct (no false positive) |
| UA-01 (dual permission architecture) | P1-01 (6 loop owners) | Related: fragmentation extends to security paths |
| UA-02 (plan approval no checks) | P1-15 (no plan approval test) | Test gap reveals un-audited security path |
| UA-03 (artifact export no secret scan) | NEW | Independent finding |
| UA-04 (concurrent tool safety) | P1-09 (TCML bypass in concurrent path) | Related: concurrent path needs safety analysis |
| UA-05 (FileReadTool no gate) | P0 positive #10 | Refines: protection at execution layer, not at gate layer |

---

## Summary

### New Vulnerabilities Found: 6
| ID | Severity | Summary |
|---|---|---|
| RED-01 | P0 | `sh`/`bash`/`zsh` interpreters not blocked at any layer |
| RED-02 | P1 | Unicode whitespace bypass for DENY_SUBSTRINGS substring matching |
| RED-03 | P1 | Native agent loop execution path skips Layer A hard deny entirely |
| RED-04 | P2 | `is_sensitive_path` coverage gap vs `check_dangerous_path` |
| RED-05 | P2 | No permission submission rate limiting |
| RED-06 | P2 | `artifact.export` has no security classifier or content scan |

### Severity Upgrades: 2
| ID | From | To | Justification |
|---|---|---|---|
| P2-17 (sudo not in Layer A) | P2 | P1 | Combined with Unicode bypass and missing shell blocking, enables full system compromise |
| P2-19 (missing dangerous programs) | P2 | P1 | `mkfs`, `dd`, `fdisk` can cause irreversible data loss |

### Severity Clarification: 1
| ID | Finding | Clarification |
|---|---|---|
| P1-17 | Classifier Deny not true deny | Execution-layer backstop exists in `command.rs:run_prepared_command`; the real risk is UX confusion, not security bypass. Keep P1. |

### False Positives Confirmed: 0
No existing audit findings were found to be incorrect or overstated. All findings in the Issue Matrix are factually accurate, though P1-17 needs clarification about the execution-layer backstop.

### Un-audited Execution Paths: 5
| ID | Path | Risk |
|---|---|---|
| UA-01 | Dual permission architecture (facade vs. native loop) | Inconsistent security guarantees |
| UA-02 | Plan approval has no automated safety checks | Bulk approval of dangerous plans |
| UA-03 | `artifact.export` lacks content inspection | Sensitive data exfiltration without redaction |
| UA-04 | Concurrent tool execution path safety | Race conditions in parallel tool execution |
| UA-05 | `FileReadTool` has no permission gate check | Gate says Allow for reads of sensitive files |
