#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

fail() {
  echo "BOUNDARY VIOLATION: $*" >&2
  exit 1
}

native_files=(crates/runtime/src/native_agent_loop_*.rs)

for file in "${native_files[@]}"; do
  case "$file" in
    *_tests.rs|*_fixtures.rs) continue ;;
  esac
  if grep -q 'use super::\*;' "$file"; then
    fail "$file still uses 'use super::*'"
  fi
  if grep -q 'pub(super)' "$file"; then
    fail "$file still exposes pub(super); use pub(in crate::native_agent_loop) or private"
  fi
  code_lines="$(grep -cE '^\s*[^/[:space:]]' "$file" || true)"
  if [[ "$code_lines" -lt 5 ]]; then
    fail "$file is an empty/placeholder sibling (code lines: $code_lines); delete it or fill it"
  fi
done

allowed_pub="$(
  sed -n '/^## 4\./,/^## 5\./p' docs/architecture/native_agent_loop_module_api.md \
    | grep -oE 'pub (fn|struct|enum) [A-Za-z_][A-Za-z0-9_]*' \
    | awk '{print $3}' \
    | sort -u
)"

actual_pub="$(
  grep -rE '^pub (fn|struct|enum) ' crates/runtime/src/native_agent_loop*.rs \
    | grep -v '_fixtures.rs' \
    | grep -v '_tests.rs' \
    | grep -oE 'pub (fn|struct|enum) [A-Za-z_][A-Za-z0-9_]*' \
    | awk '{print $3}' \
    | sort -u
)"

if ! diff -u <(printf '%s\n' "$allowed_pub") <(printf '%s\n' "$actual_pub"); then
  fail "public native_agent_loop surface drifted from docs/architecture/native_agent_loop_module_api.md §4"
fi

check_no_import() {
  local file="$1"
  local forbidden="$2"
  local hit
  hit="$(rg -n "use crate::native_agent_loop::native_agent_loop_(${forbidden})" "$file" || true)"
  if [[ -n "$hit" ]]; then
    printf '%s\n' "$hit" >&2
    fail "$file imports a forbidden native_agent_loop sibling"
  fi
}

check_no_import crates/runtime/src/native_agent_loop_util.rs \
  '(prompt|execution|model_io|tools|completion|continuation|entrypoints|resume)'
check_no_import crates/runtime/src/native_agent_loop_prompt.rs \
  '(execution|model_io|tools|completion|continuation|entrypoints|resume)'
check_no_import crates/runtime/src/native_agent_loop_execution.rs \
  '(prompt|model_io|tools|completion|continuation|entrypoints|resume)'
check_no_import crates/runtime/src/native_agent_loop_model_io.rs \
  '(model_io|tools|completion|continuation|entrypoints|resume)'
check_no_import crates/runtime/src/native_agent_loop_tools.rs \
  '(model_io|tools|completion|continuation|entrypoints|resume)'
check_no_import crates/runtime/src/native_agent_loop_completion.rs \
  '(completion|continuation|entrypoints|resume)'
check_no_import crates/runtime/src/native_agent_loop_continuation.rs \
  '(completion|continuation|entrypoints|resume)'
check_no_import crates/runtime/src/native_agent_loop_entrypoints.rs 'resume'
check_no_import crates/runtime/src/native_agent_loop_resume.rs 'entrypoints'

sibling_count="$(ls crates/runtime/src/native_agent_loop_*.rs | wc -l | tr -d ' ')"
if [[ "$sibling_count" -gt 12 ]]; then
  fail "new native_agent_loop sibling forbidden: $sibling_count files (max 12 before Phase 2.b)"
fi

echo "native_agent_loop boundary lint passed"
