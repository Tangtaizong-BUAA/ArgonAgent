# Agent 14: Security / Shell Classifier Audit

## Conclusion

The system implements defense-in-depth with three layers of shell command permission control. However, the classifier "Deny" is not a true deny (becomes user-approvable `SafetyCheck` → `Ask`), there are inconsistencies between the two gate layers, and blind spots in dangerous command coverage (notably `rmdir`).

**Severity:** P1 (classifier Deny is user-overridable; `rmdir` passes both gates unblocked)

## Files Involved

- `crates/runtime/src/runtime_facade_impl.rs` (3054, 3192-3209) — Layer A facade early check `command_contains_hard_deny`
- `crates/runtime/src/agent_kernel/permission_gate.rs` (156-232, 381, 599-663, 696-945) — Layer B classifier, `PermissionGate`, `check_dangerous_path`
- `crates/runtime/src/runtime/permission_service.rs` — facade-permission bridge, `apply_permission_policy`
- `crates/runtime/src/command.rs` (124-148) — command execution without shell
- `crates/runtime/src/tool_execution.rs` (147-151, 504-505) — execute_shell_command, execute_tool_with_permission_gate
- `crates/runtime/src/agent_kernel/permission_policy.rs` — PermissionMode enum

## Key Findings

### Finding 1: Classifier "Deny" Is Not a True Deny (P1)

Classifier `Deny` becomes `ToolPermissionResult::SafetyCheck { classifier_approvable: false }` → `PermissionResolution::Ask { safety: true, persistable: false }` → `FacadeToolMode::RequirePermission`. User CAN approve classifier-denied commands; they just can't create persistent rules. Only Layer A (`command_contains_hard_deny`) provides true non-overridable blocking.

### Finding 2: `rmdir` Not Blocked by Either Layer (P2)

`command_contains_hard_deny` checks for ` rm ` (space-delimited) and `rm ` (starts with). `rmdir dir` does NOT start with `rm `. Classifier program deny list: `["rm", "mv", "cp", "chmod", "chown", "kill", "pkill"]` — `rmdir` not included. Falls through to `Ask`.

### Finding 3: `sudo` Not in Layer A Hard Block (P2)

`sudo` is only in classifier DENY_SUBSTRINGS. `sudo chmod 777 /etc/shadow` passes Layer A (no `sudo`, `chmod`, `777` in hard deny patterns). Classifier catches `chmod` as program deny but becomes user-approvable `SafetyCheck` → `Ask`.

### Finding 4: PermissionService Creates New Gate Per Evaluation (P2)

`PermissionService::apply_permission_policy` creates `new_gate` per evaluation (line 39-53). Doc39 design comment (line 692) says gate should be "long-lived... so denial tracking is not reset on every tool call." Native agent loop path passes persistent gate, creating dual behavior.

### Finding 5: Missing Dangerous Programs (P2)

`mkfs`, `dd`, `fdisk`, `parted`, `mount`, `umount`, `shutdown`, `reboot`, `systemctl` are NOT in any deny list. `mkfs.ext4 /dev/sda` would be classified as `Ask` (user-approvable).

### Finding 6: Shell Execution Model Correct (P0 ✓)

Commands NEVER executed via shell. `tokenize_command` with quote awareness, escape handling → `std::process::Command::new(tokens[0]).args(tokens[1..])`. All shell operators blocked at classification layer.

### Finding 7: Path Traversal Protection Strong for File Tools (P0 ✓)

`check_dangerous_path` detects `..`, DOS devices, NTFS streams, 8.3 names, UNC paths, dangerous directories (.git, .ssh, etc.), dangerous files (.env, id_rsa, etc.). Not Unicode-normalization-aware.

### Finding 8: Path Traversal Weaker for Shell Commands (P2)

`check_dangerous_path` only called by file tools (FileWrite, FileEdit, PatchApply). Shell command path traversal relies entirely on DENY_SUBSTRINGS matching and absolute path check. `cat ../../../var/log/system.log` would NOT be flagged.

### Finding 9: `--force` False Positives (P3)

DENY_SUBSTRINGS `--force` matches any occurrence. `cargo test -- --force-ful-name` triggers deny → SafetyCheck/Ask. UX friction but not a security vulnerability.

## doc39 Conflict

- **Yes** (§1.8): PermissionGate should be long-lived; PermissionService creates fresh gates per evaluation
- **No** for command execution without shell and file path traversal protection

## Suggested Fix

1. Map classifier `SafetyCheck { classifier_approvable: false }` to `PermissionResolution::Deny` instead of `Ask`
2. Add `sudo` to Layer A `command_contains_hard_deny`
3. Add `rmdir`, `mkfs`, `dd`, `fdisk`, `systemctl` to program deny list
4. Fix `PermissionService::apply_permission_policy` to accept `&mut PermissionGate` or store gate on session
5. Unify DENY_SUBSTRINGS and `command_contains_hard_deny` into single configuration source

## Handoff Needed

- Security team: review complete dangerous program list for missing entries
- UX team: decide whether SafetyCheck commands should have a visible "dangerous" warning even when user-approvable
