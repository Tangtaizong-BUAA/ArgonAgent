#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "runtime facade boundary lint failed: $*" >&2
  exit 1
}

facade="crates/runtime/src/runtime_facade.rs"

[[ -f "$facade" ]] || fail "$facade not found"

line_count=$(wc -l < "$facade" | tr -d '[:space:]')
if [[ "$line_count" -gt 800 ]]; then
  fail "$facade has $line_count lines; expected <= 800"
fi

if rg -n "Mutex<|MutexGuard|\\.lock\\(" "$facade" >/tmp/runtime_facade_lock_hits.$$; then
  cat /tmp/runtime_facade_lock_hits.$$
  rm -f /tmp/runtime_facade_lock_hits.$$
  fail "$facade must not own locks or call .lock() directly"
fi
rm -f /tmp/runtime_facade_lock_hits.$$

if ! rg -n "Arc<SessionStore>|Arc<SubagentStore>|Arc<ContextService>|Arc<PermissionService>|Arc<InterruptService>" "$facade" >/tmp/runtime_facade_service_hits.$$; then
  rm -f /tmp/runtime_facade_service_hits.$$
  fail "$facade does not expose the expected service references"
fi
service_count=$(wc -l < /tmp/runtime_facade_service_hits.$$ | tr -d '[:space:]')
rm -f /tmp/runtime_facade_service_hits.$$
if [[ "$service_count" -lt 5 ]]; then
  fail "$facade exposes $service_count service references; expected 5"
fi

echo "runtime facade boundary lint passed: $facade has $line_count lines and no direct locks"
