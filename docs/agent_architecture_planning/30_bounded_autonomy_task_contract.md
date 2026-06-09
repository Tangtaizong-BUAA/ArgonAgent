# 30 Bounded Autonomy Task Contract

本文件解决的问题：定义 Codex Desktop 长任务和未来产品内 agent 长任务的自动执行边界。目标不是每一步都问，也不是无限制自动执行，而是先批准边界，边界内自动，越界停止，最后审查。

修正旧文档的方式：把“长任务能力”从愿景收敛为 `TaskContract`，并明确默认允许/禁止、日志、diff、stop conditions、review/integration 要求。

## 1. Why Bounded Autonomy

**Decision:** Long-task autonomy must be TaskContract-scoped.

**Rationale:** Coding/research agents需要连续执行，但 shell、file write、cloud model、package install、multi-agent 并行都可能越界。TaskContract 把“允许自动执行”的边界前置。

**Rule:** 边界内自动执行；越界必须停止并报告。

**Implementation Impact:** AgentSession 创建长任务时必须绑定 TaskContract，并把 violation 写入 event log。

**Eval Impact:** Eval runner 应记录 TaskContract violation、retry count、stop condition 和 final report completeness。

**Go/No-Go Impact:** 没有 TaskContract schema 和 violation handling，不能 scaffold。

## 2. Why Not Unrestricted Autonomy

Unrestricted autonomy 会导致：

- 读取 secrets；
- 安装依赖；
- 外网泄露数据；
- 删除/覆盖文件；
- 修改核心 schema；
- 多 agent 冲突；
- 无审查长时间重试；
- 生成难以追溯的 artifacts。

## 3. TaskContract Type Sketch

```ts
type TaskContract = {
  task_id: string;
  goal: string;
  scope: string;
  allowed_paths: string[];
  denied_paths: string[];
  allowed_tools: ToolName[];
  denied_tools: ToolName[];
  shell_policy: ShellPolicy;
  network_policy: NetworkPolicy;
  package_install_policy: PackageInstallPolicy;
  cloud_model_policy: CloudModelPolicy;
  max_duration_minutes: number;
  max_retries: number;
  max_parallel_agents: number;
  required_tests: string[];
  required_artifacts: string[];
  stop_conditions: string[];
  reviewer_required: boolean;
  integrator_required: boolean;
  final_report_format: FinalReportFormat;
}

type ShellPolicy = {
  allow_non_destructive_checks: boolean;
  allow_build_or_test: boolean;
  allow_package_managers: boolean;
  deny_patterns: string[];
  require_approval_patterns: string[];
}

type NetworkPolicy = {
  mode: "disabled" | "health_check_only" | "explicit_approval";
  allowed_hosts?: string[];
}

type PackageInstallPolicy = {
  mode: "deny" | "explicit_approval";
  allowed_managers?: string[];
}

type CloudModelPolicy = {
  allow_native_deepseek: boolean;
  allow_native_qwen: boolean;
  allow_compatible_provider: "never" | "manual_only" | "baseline_only";
  sensitive_data_requires_approval: boolean;
}

type FinalReportFormat = {
  changed_files: boolean;
  tests_run: boolean;
  risks: boolean;
  unresolved_questions: boolean;
  next_tasks: boolean;
}
```

## 4. Default Allowed

**Rule:** 默认允许：

- 读取项目文件；
- 修改 `docs/`；
- 修改 `spikes/`；
- 修改 `tests/fixtures/`；
- 运行非破坏性检查命令；
- 生成报告；
- 生成 eval fixtures；
- 生成小型 spike prototype。

## 5. Default Denied

**Rule:** 默认禁止：

- 读取 `.env`；
- 读取 private key / SSH key；
- 上传文件；
- 访问外网；
- 安装依赖；
- 删除文件；
- force push；
- 修改 git history；
- 修改 lock files；
- 修改 Product Kernel；
- 修改 Event schema；
- 修改 Permission Manager；
- 修改 Patch Manager；
- 修改 Model Router core；
- 修改 DeepSeek/Qwen native adapter core strategy；
- 修改 Security model；
- 修改 AGENTS.md core rules，除非任务明确授权。

## 6. Required Fields

| Field | Rule |
|---|---|
| `goal` | Must be concrete and testable. |
| `scope` | Must name what is in/out. |
| `allowed_paths` | Empty means read-only; write requires explicit path. |
| `denied_paths` | Must include secrets/protected/core paths by default. |
| `allowed_tools` | Must exclude install/network/destructive tools unless approved. |
| `denied_tools` | Must include package install and network by default. |
| `max_retries` | Default 2 for implementation, 1 for spike, 0 for docs-only if not needed. |
| `max_parallel_agents` | Default 1. |
| `required_tests` | Must exist for implementation/spike. |
| `required_artifacts` | Docs/reports/fixtures/scripts expected at finish. |

## 7. Stop Conditions

Agent must stop and report if:

- required path is outside `allowed_paths`;
- a denied path/tool is needed;
- external network is required;
- package install is required;
- secret/protected path appears;
- Product Kernel/Event schema/security/native adapter core would need changes without authorization;
- retry budget is exceeded;
- test failure cannot be diagnosed within scope;
- multi-agent conflict touches shared files;
- model/provider capability contradicts the contract.

## 8. Review Boundaries

**Rule:** No-review execution is allowed only for docs/prototypes inside explicitly allowed paths and no security-sensitive changes.

Reviewer is required when:

- changing schemas;
- changing security/threat docs;
- changing native model strategy;
- changing eval promotion gates;
- producing implementation code;
- using multi-agent outputs.

Integrator is required when:

- more than one agent produced artifacts;
- multiple docs define overlapping contracts;
- spike output may influence product architecture.

## 9. Logs and Diff Requirements

Every autonomous task must report:

- TaskContract summary;
- files read/changed at high level;
- commands run;
- tests/checks run;
- generated artifacts;
- stop/violation events;
- risks and open questions.

Every file write must be reviewable through diff or artifact hash. Product code writes require PatchProposal path/base-hash validation.

## 10. Codex Desktop Usage

**Rule:** Codex long tasks can run without step-by-step user confirmation only inside TaskContract.

Recommended:

- Use single main thread for architecture contracts.
- Use separate Codex threads only for read-only review or isolated spike.
- Put each spike in a separate directory.
- Require final report from every thread.

## 10.1 Plan Completion Mandate for Active Backlogs

**Decision:** If the user explicitly asks to complete the full plan/backlog,
the active plan becomes the TaskContract seed.

**Rule:** The agent should infer a bounded implementation contract from the
latest accepted plan/backlog and continue through all unblocked items without
asking the user to say "continue" between slices.

**Inferred contract defaults:**

- **Goal:** complete every unblocked item in the active plan/backlog.
- **Scope:** repository implementation, tests, fixtures, docs, scripts, and
  runtime/TUI harness work needed by that plan.
- **Allowed paths:** source files, tests, fixtures, docs, scripts, and local
  artifacts under project-controlled directories.
- **Denied paths:** `.env`, private keys, SSH keys, unrelated user files,
  dependency lockfiles unless explicitly in scope, git history, and destructive
  cleanup paths.
- **Allowed tools:** read/search/edit/patch, non-destructive local checks,
  focused tests, broad harness tests, `git status`, `git diff`, `git add`, and
  checkpoint commits.
- **Denied tools:** destructive shell, force push, git history rewrite, package
  install, network upload, and secret reads.
- **Stop conditions:** denied boundary, dependency install requirement, network
  requirement not already authorized, repeated unresolved test failure,
  architecture contradiction, or work beyond the active plan.

**Implementation Impact:** Long-running Codex sessions should maintain a
checklist, mark blocked items with evidence, keep executing other unblocked
items, and only send the final report after all unblocked items are done or a
true stop condition is reached.

**Eval Impact:** Harnesses should treat premature final reports with remaining
unblocked tasks as failures.

## 10.2 Engineering Completion Gate

**Decision:** A TaskContract is complete only when its requested capability is
implemented to engineering quality or a true stop condition is reached.

**Rule:** Smoke tests, placeholder scaffolds, docs-only descriptions, or
partially wired APIs do not satisfy an implementation TaskContract.

**Required completion evidence:**

- implemented runtime path;
- CLI/TUI path wired through the same RuntimeFacade/EventLog contract intended
  for GUI reuse;
- structured error handling for unsupported tools, failed parser output,
  permission denial, patch conflicts, tool failures, and session recovery;
- DeepSeek/Qwen native behavior preserved separately from compatible provider
  behavior;
- focused tests for the changed slice;
- broad harness checks or a documented blocker;
- reproducible command or fixture;
- replayable event log when the change touches runtime/session/tool/model
  behavior;
- explicit report of remaining risk and blocked items.

**Implementation Impact:** Agents must continue after partial scaffolding. A
final report that leaves requested unblocked implementation work incomplete is a
contract failure.

**Go/No-Go Impact:** The project cannot claim ClaudeCode/OpenCode parity for a
subsystem until this gate passes for that subsystem.

## 11. Product Usage

In the product, TaskContract becomes:

- session creation policy;
- GUI task boundary preview;
- permission escalation baseline;
- multi-agent scheduler input;
- event log audit anchor.

## 12. Example: Documentation Task

```ts
const docTask: TaskContract = {
  task_id: "phase0-doc-consolidation",
  goal: "Update architecture docs to reflect DeepSeek/Qwen native-first scope",
  scope: "Docs only; no product code",
  allowed_paths: ["docs/agent_architecture_planning/", "docs/security/", "docs/implementation/"],
  denied_paths: [".env", "~/.ssh", "src/", "crates/", "package-lock.json"],
  allowed_tools: ["read", "rg", "sed", "apply_patch", "python_text_check"],
  denied_tools: ["network", "package_install", "destructive_shell"],
  shell_policy: {
    allow_non_destructive_checks: true,
    allow_build_or_test: false,
    allow_package_managers: false,
    deny_patterns: ["rm", "git reset", "git push"],
    require_approval_patterns: []
  },
  network_policy: { mode: "disabled" },
  package_install_policy: { mode: "deny" },
  cloud_model_policy: {
    allow_native_deepseek: false,
    allow_native_qwen: false,
    allow_compatible_provider: "never",
    sensitive_data_requires_approval: true
  },
  max_duration_minutes: 180,
  max_retries: 1,
  max_parallel_agents: 1,
  required_tests: ["rg consistency checks"],
  required_artifacts: ["updated markdown docs"],
  stop_conditions: ["requires product code", "requires network", "requires secret file"],
  reviewer_required: false,
  integrator_required: false,
  final_report_format: { changed_files: true, tests_run: true, risks: true, unresolved_questions: true, next_tasks: true }
}
```

## 13. Example: Spike Task

```ts
const spikeTask: TaskContract = {
  task_id: "deepseek-parser-fixture-spike",
  goal: "Create parser golden fixtures for DeepSeek XML/JSON outputs",
  scope: "Fixture data only under eval/fixtures/deepseek",
  allowed_paths: ["eval/fixtures/deepseek/", "docs/prototypes/"],
  denied_paths: ["src/", "docs/agent_architecture_planning/21_product_kernel_v0.md"],
  allowed_tools: ["read", "rg", "apply_patch", "python_text_check"],
  denied_tools: ["network", "package_install", "shell_execute_models"],
  shell_policy: {
    allow_non_destructive_checks: true,
    allow_build_or_test: false,
    allow_package_managers: false,
    deny_patterns: ["rm", "curl", "wget"],
    require_approval_patterns: []
  },
  network_policy: { mode: "disabled" },
  package_install_policy: { mode: "deny" },
  cloud_model_policy: {
    allow_native_deepseek: false,
    allow_native_qwen: false,
    allow_compatible_provider: "never",
    sensitive_data_requires_approval: true
  },
  max_duration_minutes: 90,
  max_retries: 1,
  max_parallel_agents: 2,
  required_tests: ["python3 -m json.tool eval/fixtures/deepseek/parser_golden.json"],
  required_artifacts: ["parser_golden.json"],
  stop_conditions: ["needs live model output", "needs network"],
  reviewer_required: true,
  integrator_required: true,
  final_report_format: { changed_files: true, tests_run: true, risks: true, unresolved_questions: true, next_tasks: true }
}
```

## 14. Example: Implementation Task

```ts
const implTask: TaskContract = {
  task_id: "prototype-patch-validator",
  goal: "Implement a standalone patch validator prototype against fixtures",
  scope: "Prototype script only, not product runtime",
  allowed_paths: ["scripts/prototype_patch_validator.py", "eval/fixtures/patch/"],
  denied_paths: ["src/", "crates/", "docs/agent_architecture_planning/21_product_kernel_v0.md"],
  allowed_tools: ["read", "rg", "apply_patch", "python_text_check", "python_script_run"],
  denied_tools: ["network", "package_install", "destructive_shell"],
  shell_policy: {
    allow_non_destructive_checks: true,
    allow_build_or_test: true,
    allow_package_managers: false,
    deny_patterns: ["rm", "git reset", "git push"],
    require_approval_patterns: []
  },
  network_policy: { mode: "disabled" },
  package_install_policy: { mode: "deny" },
  cloud_model_policy: {
    allow_native_deepseek: false,
    allow_native_qwen: false,
    allow_compatible_provider: "never",
    sensitive_data_requires_approval: true
  },
  max_duration_minutes: 120,
  max_retries: 2,
  max_parallel_agents: 1,
  required_tests: ["python3 scripts/prototype_patch_validator.py eval/fixtures/patch"],
  required_artifacts: ["script", "test output"],
  stop_conditions: ["needs product runtime", "needs dependency install"],
  reviewer_required: true,
  integrator_required: false,
  final_report_format: { changed_files: true, tests_run: true, risks: true, unresolved_questions: true, next_tasks: true }
}
```

## 15. Violation Handling

| Violation | Required action |
|---|---|
| Denied path needed | Stop and report. |
| Network needed | Stop unless contract permits explicit approval. |
| Package install needed | Stop and create package approval request if authorized. |
| Core schema change needed | Stop; requires new TaskContract. |
| Retry exceeded | Stop; report failure signature. |
| Multi-agent conflict | Stop parallel work; Integrator resolves. |
| Secret detected | Stop; redact; do not persist raw secret. |

## Execution Impact

- Add TaskContract schema before scaffold.
- Use TaskContract in Phase 0 execution order.
- Add TaskContract violation to threat control matrix.

## Next Tasks

1. Draft `TaskContract` JSON Schema.
2. Add examples under docs/prototypes.
3. Add a lightweight validation script in Phase 0.

## Open Questions

- Should TaskContract be signed/hashed in event log from v0?
- Should user be able to save reusable TaskContract templates per project?
