PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS projects (
  project_id TEXT PRIMARY KEY,
  path TEXT NOT NULL,
  display_name TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  session_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(project_id),
  model_profile_id TEXT NOT NULL,
  state TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
  event_id TEXT PRIMARY KEY,
  schema_version TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(project_id),
  session_id TEXT REFERENCES sessions(session_id),
  task_id TEXT,
  sequence INTEGER NOT NULL,
  event_type TEXT NOT NULL,
  actor TEXT NOT NULL,
  created_at TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  prev_hash TEXT,
  hash TEXT NOT NULL,
  UNIQUE(project_id, session_id, sequence)
);

CREATE TABLE IF NOT EXISTS plan_approvals (
  plan_approval_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(session_id),
  plan_id TEXT NOT NULL,
  request_event_id TEXT NOT NULL REFERENCES events(event_id),
  decision_event_id TEXT REFERENCES events(event_id),
  status TEXT NOT NULL,
  request_hash TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS permissions (
  permission_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(session_id),
  request_type TEXT NOT NULL CHECK (request_type IN ('command', 'file_write', 'network', 'package_install', 'cloud_model', 'protected_path', 'artifact_export')),
  request_event_id TEXT NOT NULL REFERENCES events(event_id),
  decision_event_id TEXT REFERENCES events(event_id),
  status TEXT NOT NULL,
  request_hash TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_calls (
  tool_call_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(session_id),
  tool_id TEXT NOT NULL,
  request_event_id TEXT NOT NULL REFERENCES events(event_id),
  result_event_id TEXT REFERENCES events(event_id),
  status TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS model_calls (
  model_call_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(session_id),
  model_profile_id TEXT NOT NULL,
  provider_id TEXT,
  started_event_id TEXT NOT NULL REFERENCES events(event_id),
  completed_event_id TEXT REFERENCES events(event_id),
  prompt_template_version TEXT,
  parser_version TEXT,
  token_input INTEGER,
  token_output INTEGER,
  token_reasoning INTEGER
);

CREATE TABLE IF NOT EXISTS patches (
  patch_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(session_id),
  proposed_event_id TEXT NOT NULL REFERENCES events(event_id),
  applied_event_id TEXT REFERENCES events(event_id),
  target_paths_json TEXT NOT NULL,
  base_hashes_json TEXT NOT NULL,
  status TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS artifacts (
  artifact_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(project_id),
  session_id TEXT REFERENCES sessions(session_id),
  kind TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  mime_type TEXT,
  logical_name TEXT NOT NULL,
  source_event_id TEXT NOT NULL REFERENCES events(event_id),
  privacy_class TEXT NOT NULL,
  retention_policy TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS research_jobs (
  research_job_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(project_id),
  session_id TEXT REFERENCES sessions(session_id),
  state TEXT NOT NULL,
  manifest_artifact_id TEXT REFERENCES artifacts(artifact_id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS eval_results (
  eval_result_id TEXT PRIMARY KEY,
  eval_run_id TEXT NOT NULL,
  eval_case_id TEXT NOT NULL,
  fixture_hash TEXT NOT NULL,
  model_profile_id TEXT NOT NULL,
  metric_name TEXT NOT NULL,
  metric_value TEXT NOT NULL,
  verdict TEXT,
  event_id TEXT REFERENCES events(event_id)
);

