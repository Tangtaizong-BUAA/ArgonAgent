# Threat Control Matrix

本文件解决的问题：把 `docs/agent_architecture_planning/19_threat_model.md` 转成工程控制矩阵，明确 release-blocking rules、implementation task、eval/test case 和当前状态。

修正旧文档的方式：threat model 原来是风险分析；本文件把风险变成阻塞发布的控制项。

## Release-Blocking Rules

**Decision:** 以下规则任一失败，不能进入 scaffold 或 release：

- denied command must never execute;
- patch with stale base hash must never apply;
- cloud model call with sensitive data must require approval;
- protected path write must require approval;
- compatible provider cannot be marked native;
- multi-agent cannot modify kernel concurrently;
- TaskContract violation must stop execution;
- DeepSeek parser wrong-tool execution blocks promotion;
- Qwen parser wrong-tool execution blocks promotion;
- repaired tool args must not execute if repair confidence is low;
- reasoning traces containing secrets must not be persisted.

## Control Matrix

| # | Threat | Severity | Likelihood | Priority | Control | Implementation task | Eval/test case | Release-blocking rule | Current status |
|---:|---|---|---|---|---|---|---|---|---|
| 1 | repo prompt injection | High | High | P0 | Context trust labels; repo text as data; deny network/secrets | Kernel schema + context bundle spec | C-08, DS-04 | Injection must not trigger secret read/network | Spec only |
| 2 | secret exfiltration | Critical | Medium | P0 | Secret path deny; redaction before model/persistence | Permission contract; secret detector spike | Secret fixture | Secrets must not enter cloud/model logs | Spec only |
| 3 | cloud model data leakage | Critical | Medium | P0 | Privacy classification; cloud approval | Research worker privacy + Plan/Permission split | R-04 | Sensitive cloud call requires approval | Spec only |
| 4 | shell command injection | Critical | High | P0 | Command parser/classifier; exact preview; deny risky segments | Command permission classifier spike | S-03, shell injection cases | Denied command never executes | Spec only |
| 5 | destructive file write | Critical | Medium | P0 | PatchProposal, read-before-write, protected paths | Patch validator spike | P-01..P-05 | Stale/protected patch never applies | Spec only |
| 6 | package install risk | High | Medium | P0 | Separate package install approval; no install by default | Permission UI contract | S-04 | Package install requires explicit approval | Spec only |
| 7 | Python research worker leakage | Critical | Medium | P0 | Sidecar sandbox; no network; mounted inputs/outputs | Research CSV profiler + worker manifest | R-01..R-05 | Worker cannot read outside mounts or use network by default | Spec only |
| 8 | worktree merge corruption | High | Medium | P1 | Worktree isolation; merge preview; tests after merge | Later worktree spike | Multi-agent merge fixture | Multi-agent product code requires worktree | Deferred |
| 9 | GUI approval spoofing | High | Medium | P0 | Show raw/normalized request, paths, risk, rule, hash | Permission UI contract prototype | UI approval snapshot | Approval must bind exact request hash | Spec only |
| 10 | local API abuse | High | Medium | P1 | Tauri IPC first; local API token/origin spike | Local API auth/streaming spike | API auth test | Local API cannot run without auth | Deferred |
| 11 | event log tampering | High | Low | P1 | Hash chain; append-only events; integrity check | Event log replay spike | Event sequence validator | Event hash chain must verify | Spec only |
| 12 | compatible provider misconfiguration | High | Medium | P0 | ProviderConfig validation; optimization_level enum | ProviderConfig/ModelAlias spike | Provider validation cases | Compatible provider cannot be native | Spec only |
| 13 | model alias confusion | Medium | High | P0 | Show base_url/actual/display/alias; health check | ModelAlias schema spike | Alias mismatch case | actual_model_name must be visible/logged | Spec only |
| 14 | multi-agent conflicting modifications | High | Medium | P0 | Multi-agent policy; explicit write scopes; Integrator | TaskContract schema | Multi-agent conflict fixture | Multi-agent cannot modify kernel concurrently | Spec only |
| 15 | no-review long task runaway | High | Medium | P0 | TaskContract; max retries/duration/stop conditions | TaskContract schema | TaskContract violation test | Contract violation stops execution | Spec only |
| 16 | DeepSeek parser wrong-tool execution | Critical | Medium | P0 | Native parser confidence; retry/deny on ambiguity | DeepSeek parser fixture spike | DS-01/DS-02 | Wrong-tool execution blocks promotion | Spec only |
| 17 | Qwen executor hallucinated file modification | Critical | Medium | P0 | Patch-sized edits; stale-file detection; reviewer/test loop | Qwen executor fixtures | Qwen coding/patch eval | Hallucinated file edit blocks executor promotion | Spec only |
| 18 | sensitive research data exfiltration | Critical | Medium | P0 | PII detection; cloud approval; no-network worker | Research CSV profiler | R-04 | Sensitive rows not sent before approval | Spec only |
| 19 | artifact retention privacy | High | Medium | P1 | Artifact privacy class; retention policy; delete/export events | Artifact store spec | Retention test | Sensitive artifact export requires approval | Spec only |
| 20 | plugin/skill abuse | High | Medium | P2 | Plugins out of kernel v0; signed/gated later | Future plugin threat eval | Future plugin cases | Plugin cannot bypass permissions | Deferred |
| 21 | DeepSeek reasoning trace leakage | Critical | Medium | P0 | Reasoning sanitizer; retention policy; secret redaction | DeepSeek reasoning fixture | DS-04 | Reasoning traces containing secrets not persisted | Spec only |
| 22 | Qwen parser/template mismatch | High | Medium | P0 | Deployment capability check; parser flags logged | Qwen parser/executor fixture | Qwen parser gate | Qwen native blocked if parser/template missing | Spec only |
| 23 | prefix-cache optimization leaking stale context | High | Low | P1 | Prefix hash, context source labels, compaction boundary tests | DeepSeek context fixture | DS-03/DS-05 | Cache policy cannot replay stale/sensitive context | Spec only |
| 24 | repaired tool call executing wrong args | Critical | Medium | P0 | Repair confidence threshold; low confidence deny/retry | Parser repair fixtures | DS-02, Qwen parser cases | Low-confidence repaired args never execute | Spec only |

## Engineering Decisions

- **Decision:** All P0 rows must have at least one Phase 0 schema/spec/fixture task before scaffold.
- **Rule:** Current status `Spec only` is acceptable for architecture convergence, not for scaffold.
- **Implementation Impact:** Phase 0 execution order must cover rows 1-7, 12-18, 21-24.
- **Eval Impact:** Native parser/executor promotion is blocked until DeepSeek/Qwen fixture gates pass.
- **Go/No-Go Impact:** Scaffold is blocked until the first P0 controls have runnable spikes or accepted deferrals.

## Open Questions

- Which P0 controls must be executable scripts before scaffold versus schema/fixture specs?
- Should event log hash-chain validation be required before any GUI prototype?

