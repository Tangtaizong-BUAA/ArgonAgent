# 09 Research Coworker Architecture

## Goal

Make research/data workflows a first-class peer to coding tasks:
- organize experiment folders;
- profile and clean datasets;
- generate and run scripts/notebooks;
- produce charts/reports;
- extract literature/methods;
- support reproducibility.

## 1. Research Project Workspace

Structure:

```text
.researchcode/
  research/
    index.sqlite
    jobs/
    artifacts/
    notebooks/
    reports/
    scripts/
    cache/
```

Responsibilities:
- Index project datasets, papers, scripts, notebooks, configs, results, and reports.
- Keep research artifacts separate from source code until user decides to commit.

## 2. Data File Index

Supported:
- CSV, TSV, Excel, JSON/JSONL, Parquet, Feather, SQLite, images metadata, folder trees.

Index fields:
- path, type, size, modified time, hash, row/column estimates, sample preview, sensitivity flag, related jobs.

Tools:
- `data_index_scan`
- `data_sample`
- `data_schema_profile`
- `data_lineage_trace`

## 3. Schema Profiler

Implementation:
- Python sidecar with Pandas/Polars/DuckDB.
- For large data, sample first then full stats on demand.

Outputs:
- column names/types;
- null rate;
- distinct count;
- min/max/quantiles;
- examples;
- inferred keys;
- date/time parsing candidates;
- categorical distribution.

## 4. Data Quality Checks

Checks:
- missing values;
- duplicate rows/keys;
- type drift;
- invalid dates;
- outliers;
- inconsistent units;
- encoding issues;
- suspicious labels/leakage;
- train/test contamination;
- broken file references.

Output:
- `DataQualityReport` artifact plus structured findings.

## 5. Cleaning Plan

Flow:
- profiler -> model-generated cleaning plan -> user approval -> generated Python script -> execution -> validation report.

Rules:
- Never overwrite original data.
- Write cleaned outputs under `artifacts/cleaned/`.
- Store transformations as script and metadata.

## 6. Python Script Generation and Sandbox Execution

Python sidecar:
- runs in project-scoped venv/uv environment or managed bundled Python;
- limits cwd, network, and output paths;
- captures stdout/stderr, images, tables, errors;
- records dependency imports.

Execution result:
- script path, exit status, runtime, logs, generated artifacts.

Sandbox hard requirements:
- cwd is restricted to the research job workspace and approved input paths.
- outputs are restricted to `.researchcode/research/artifacts/` unless the user approves a write elsewhere.
- network defaults to deny; package installation and remote data access require `PermissionRequest`.
- CPU time, wall time, memory, output file count, and output byte limits are enforced per job.
- environment snapshot records Python version, package lock/uv lock, platform, command, input hashes, and relevant env vars with secrets redacted.
- original datasets are read-only; every derived dataset stores source hash, transformation script hash, parameters, and lineage metadata.
- PII/sensitive-column detection runs before model calls; sensitive samples are masked in context unless the user explicitly approves disclosure.
- notebook/script execution is reproducible from a generated `run_manifest.json`.

## 7. Notebook Generation

Capabilities:
- Generate `.ipynb` with markdown narrative, code cells, charts.
- Export notebook to Markdown/PDF later.
- Re-run notebook cells through sidecar.

Recommendation:
- Store canonical script plus notebook. Script is testable; notebook is presentation.

## 8. Chart Artifacts

Supported:
- Matplotlib/Seaborn/Plotly/Altair.

Artifacts:
- PNG/SVG/HTML plus chart spec metadata.

GUI:
- chart gallery;
- provenance shows script, input data, filters, model call.

## 9. Reports

Outputs:
- Markdown first;
- PDF/LaTeX/docx later.

Report sections:
- objective;
- data sources;
- cleaning decisions;
- analysis methods;
- results;
- limitations;
- reproducibility steps;
- artifact links.

## 10. Experiment Folder Organizer

Tools:
- `experiment_index`
- `experiment_metadata_extract`
- `experiment_readme_generate`
- `experiment_compare_runs`

Metadata table:
- run_id, date, config path, dataset hash, code commit, metrics, output files, notes.

## 11. Literature / Paper Parser

Capabilities:
- Parse PDF/Markdown/LaTeX.
- Extract methods, datasets, baselines, metrics, equations, limitations, reproduction steps.
- Link papers to experiments and reports.

Implementation:
- Local PDF text extraction first.
- Optional OCR and web metadata later.

## 12. Reproducibility Assistant

Workflow:
1. Identify environment files.
2. Resolve commands/scripts.
3. Build run plan.
4. Execute in sandbox/worktree.
5. Capture logs and metrics.
6. Compare results to expected.
7. Write reproducibility report.

## 13. Research Memory

Memory items:
- dataset facts;
- cleaning decisions;
- experiment conventions;
- paper notes;
- model failure modes;
- team preferences.

Privacy:
- default project-local;
- no cloud sync unless explicitly enabled;
- sensitive columns/paths flagged.

## 14. Team Collaboration Extension

Later:
- shared research project dashboard;
- artifact comments;
- review/approval for reports and data cleaning;
- role-based access to datasets;
- audit logs for data access.

## 15. Reused Coding-Agent Modules

Reuse:
- Task/session/event log.
- Tool registry and permissions.
- Patch manager for scripts/reports.
- Worktree manager for reproducible code changes.
- Model router.
- Eval harness.
- Memory system.

## 16. Error Handling

Common failures:
- parse error;
- out-of-memory;
- dependency missing;
- encoding issue;
- wrong delimiter;
- chart rendering failure;
- notebook execution timeout;
- data privacy violation.

Recovery:
- sample data;
- switch Pandas -> Polars/DuckDB;
- ask user for delimiter/schema;
- install dependency with approval;
- reduce chart data;
- write diagnosis artifact.

Non-recoverable by automation:
- request to overwrite original data;
- model request containing unapproved sensitive columns;
- script requesting network or shell escape without permission;
- dependency install that changes a locked environment without approval.

## 17. First Version

Build:
- dataset index and sample preview;
- schema/quality profiling for CSV/Excel/JSON/Parquet;
- Python script generation/execution;
- chart artifact capture;
- Markdown report generation;
- GUI research workspace with job timeline.

## 18. Tests

Tests:
- fixture datasets with known profiles;
- dirty data cleaning eval;
- script generation smoke test;
- notebook execution roundtrip;
- chart artifact existence and non-empty image check;
- privacy tests for secret/sensitive fields.
