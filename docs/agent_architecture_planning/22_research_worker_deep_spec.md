# 22 Research Worker Deep Spec

Research Worker is a first-class subsystem, but it must not weaken Product Kernel v0. The runtime owns permissions, event log, model calls, artifact store, and privacy policy. The Research Worker executes bounded data jobs from explicit manifests and returns artifacts with hashes, lineage, and validation results.

## 1. Responsibilities

Research Worker handles:

- CSV, TSV, Excel, JSON, JSONL, Parquet, Arrow, and directory-level data indexing.
- Schema profiling and data quality checks.
- Generated Python script execution.
- Notebook, chart, report, and reproducibility manifest creation.
- Experiment folder indexing and metadata extraction.
- Privacy classification and PII/sensitive-column detection.

Research Worker does **not**:

- Decide cloud model routing.
- Approve package installs.
- Read arbitrary files outside declared mounts.
- Persist untracked artifacts.
- Bypass event logging.

## 2. ResearchJob Lifecycle

| State | Trigger | Worker Action | Runtime/Event Output |
|---|---|---|---|
| `Drafted` | Agent/user proposes analysis. | None. | `research.job_drafted` |
| `ClassifyingData` | Runtime scans inputs. | File metadata only unless approved. | `research.data_classified` |
| `AwaitingApproval` | Sensitive data, package, or network needed. | None. | `permission.requested` |
| `Prepared` | Manifest approved. | Create isolated env/workdir. | `research.job_prepared` |
| `Running` | Runtime starts sidecar. | Execute script/notebook step. | `research.worker_started`, progress events |
| `CollectingArtifacts` | Script exits. | Hash outputs, validate chart/report. | `artifact.created` |
| `Completed` | All artifacts valid. | Emit manifest. | `research.job_completed` |
| `Failed` | Error, timeout, policy denial. | Stop process, preserve logs. | `research.job_failed` |
| `Cancelled` | User/runtime cancel. | Kill process, collect partial logs. | `research.job_cancelled` |

## 3. Data Lineage

Every derived output must trace to declared inputs, script version, environment, and runtime event ids.

```ts
type DataLineage = {
  lineage_id: string
  research_job_id: string
  source_artifacts: string[]
  source_file_hashes: Record<string, string>
  transformation_script_artifact_id: string
  environment_id: string
  parameters_json: unknown
  output_artifact_ids: string[]
  created_from_event_id: string
}
```

### Lineage Rules

- Raw input files are referenced by hash and path; large files may use sampled hash plus size/mtime if full hashing is deferred.
- Every generated chart/report/notebook/script links back to `ResearchJob`.
- Manual user edits to reports/notebooks become new artifacts, not mutations of old ones.
- Cloud model summaries must record which data profile or sample was sent.

## 4. Artifact Hashes

| Artifact Kind | Hash Requirement | Extra Metadata |
|---|---|---|
| Raw dataset reference | sha256 if <= configured threshold; otherwise staged hash manifest | size, mtime, path, privacy class |
| Data profile | sha256 | row count, column count, sample policy |
| Generated script | sha256 | interpreter, packages, parameters |
| Notebook | sha256 | cell count, execution status |
| Chart | sha256 | chart type, data source, validation status |
| Report | sha256 | source artifacts, model profile, citations |
| Reproducibility manifest | sha256 | environment lock, input hashes, command |

## 5. Python Environment Management

### v0 Strategy

- Use one managed base Python runtime plus per-job virtual environments.
- Default packages: `pandas`, `polars`, `duckdb`, `pyarrow`, `openpyxl`, `matplotlib`, `seaborn`, `plotly`, `jinja2`, `nbformat`.
- Package installs require `package_install` PermissionRequest.
- Environment is identified by `environment_id = hash(python_version + package_lock + platform)`.

### Environment Manifest

```json
{
  "python_version": "3.x",
  "packages": [{"name": "pandas", "version": "pinned"}],
  "network": "disabled",
  "created_at": "timestamp",
  "lock_hash": "sha256"
}
```

### Package Install Approval

PermissionRequest must show:

- Package name, version/range, registry URL.
- Whether install scripts/native extensions are present where detectable.
- Reason requested by agent.
- Whether package is already in a trusted allowlist.
- Whether the dataset privacy class permits network access.

## 6. Sandbox Limits

| Limit | v0 Default | Reason |
|---|---:|---|
| Network | disabled | Prevent data exfiltration. |
| Read paths | declared input mounts only | Prevent secret/path traversal. |
| Write paths | job output directory only | Preserve artifact traceability. |
| CPU time | configurable, default 5 minutes | Avoid runaway jobs. |
| Memory | configurable, default 2GB | Protect desktop app. |
| Process tree | killed on timeout/cancel | Avoid orphan workers. |
| Stdout/stderr | capped inline, spill to artifact | Protect event log/context. |

OS-specific sandbox implementations must report `sandbox_status` in events. If the runtime cannot enforce a limit on a platform, the GUI must display that downgrade before running.

## 7. Data Privacy Classification

```ts
type PrivacyClass =
  | "public"
  | "internal"
  | "confidential"
  | "sensitive_personal"
  | "secret"
```

### Classification Inputs

- Path patterns: `.env`, credentials, patient data folders, finance folders.
- Column names: email, phone, name, address, ssn, id, patient, subject, token, key.
- Value patterns: emails, phone numbers, SSN-like strings, API key formats, high-cardinality identifiers.
- User/project policy overrides.
- File metadata and provenance.

### Policy Defaults

- `secret`: never send to cloud model; read requires explicit approval.
- `sensitive_personal`: cloud model call requires explicit approval and sample minimization.
- `confidential`: cloud approval depends on project policy.
- `internal/public`: cloud allowed if provider policy permits.

## 8. PII / Sensitive Column Detection

### Detector Output

```ts
type ColumnSensitivity = {
  column_name: string
  inferred_type: string
  sensitivity: PrivacyClass
  evidence: string[]
  sample_policy: "none" | "masked" | "small_sample" | "full_allowed"
  confidence: "low" | "medium" | "high"
}
```

### Required Checks

- Column-name heuristics.
- Regex patterns over sampled values.
- Cardinality ratio and uniqueness.
- Date/location/person-like columns.
- Free-text column warning.
- User override event if classification is changed.

## 9. Notebook / Script / Report Lifecycle

| Artifact | Draft | Execution | Review | Finalization |
|---|---|---|---|---|
| Script | Agent proposes code as artifact. | Worker runs with manifest. | Runtime checks outputs and errors. | Saved with hash and environment id. |
| Notebook | Generated from script or template. | Optional execution in worker. | Cell outputs validated. | Exported as `.ipynb` artifact. |
| Report | Markdown generated from profiles/charts. | No direct code execution. | Citations/artifact refs checked. | Exported as `.md` and optional PDF later. |
| Chart | Generated by script. | Rendered to PNG/SVG/HTML. | Chart validator checks non-empty, labels, data link. | Stored as artifact with preview metadata. |

## 10. Chart Validation

Chart artifacts must be checked before being presented as successful:

- File exists and size above minimum threshold.
- Image can be opened or HTML contains expected plot payload.
- Axes/title/legend requirements based on task.
- Underlying data row count matches manifest.
- No sensitive raw values appear in labels unless approved.
- Generated preview hash stored.

## 11. Reproducibility Manifest

Every completed ResearchJob emits:

```json
{
  "job_id": "...",
  "created_at": "...",
  "inputs": [{"path": "...", "sha256": "...", "privacy_class": "..."}],
  "environment": {"python": "...", "packages": [], "lock_hash": "..."},
  "commands": [],
  "scripts": ["artifact_id"],
  "outputs": ["artifact_id"],
  "random_seed": 0,
  "runtime_limits": {},
  "model_profile": "deepseek-v4-native",
  "event_log_range": ["event_id_start", "event_id_end"]
}
```

## 12. Integration with Agent Event Log

Required events:

- `research.job_drafted`
- `research.data_classified`
- `research.schema_profile_created`
- `research.script_proposed`
- `permission.requested`
- `permission.decided`
- `research.worker_started`
- `research.worker_progress`
- `research.worker_stderr`
- `research.worker_completed`
- `artifact.created`
- `research.lineage_created`
- `research.job_completed`
- `research.job_failed`

Large stdout/stderr becomes `command_output` artifacts; inline events contain summaries and artifact refs.

## 13. Model Interaction Policy

The model should receive:

- Data profile summaries.
- Masked samples where policy permits.
- Schema and quality issue summaries.
- Artifact refs and chart previews.

The model should not receive by default:

- Full raw datasets.
- Secret/sensitive columns.
- Unredacted PII.
- Full notebook outputs containing sensitive rows.

## 14. Failure Handling

| Failure | Runtime Behavior |
|---|---|
| Script error | Capture stderr, classify, allow repair loop with same manifest unless package/network change needed. |
| Timeout | Kill process tree, store partial logs, ask model for lower-cost plan. |
| Memory limit | Recommend DuckDB/Polars streaming; no automatic limit escalation. |
| Package missing | Create package approval request, do not auto-install. |
| Sensitive data detected late | Stop cloud calls, reclassify artifacts, ask user. |
| Chart invalid | Mark job partial; ask model to repair chart script. |
| Hash mismatch | Treat input as changed; require re-profile. |

## 15. First-Version Implementation Boundary

v0 should implement:

- File profiling for CSV/Excel/Parquet/JSON.
- Sidecar Python job with no network.
- Artifact hashing and lineage manifest.
- Markdown report and static chart generation.
- PII column detector v0.
- Event log integration.

v0 should not implement:

- Team sharing.
- Cloud notebook execution.
- Arbitrary package installation without approval.
- Browser scraping.
- Fully interactive notebook UI.
- Plugin research skills.

