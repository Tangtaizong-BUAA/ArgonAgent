"""Research Worker command entrypoint."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

from research_worker.artifacts import (
    write_analysis_script,
    write_markdown_report,
    write_notebook,
)
from research_worker.csv_profile import write_privacy_report, write_profile
from research_worker.manifest import ResearchJobManifest


def stable_content_hash(path: Path) -> str:
    value = 0xCBF29CE484222325
    for byte in path.read_bytes():
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"fnv64_{value:016x}"


def write_job_manifest(
    job_id: str,
    input_path: Path,
    artifact_paths: list[tuple[str, Path]],
    output_dir: Path,
    os_sandbox: dict[str, object] | None = None,
) -> Path:
    manifest = ResearchJobManifest(
        job_id=job_id,
        input_paths=(str(input_path),),
        output_dir=str(output_dir),
    )
    manifest.validate()
    input_hash = stable_content_hash(input_path)
    artifact_records = [
        {
            "artifact_kind": artifact_kind,
            "path": str(artifact_path),
            "content_hash": stable_content_hash(artifact_path),
            "source_input_hash": input_hash,
        }
        for artifact_kind, artifact_path in artifact_paths
    ]
    payload = {
        "schema_version": "research_job_manifest.v0",
        "job_id": manifest.job_id,
        "network_enabled": manifest.network_enabled,
        "privacy_class": manifest.privacy_class,
        "resource_limits": {
            "max_input_bytes": manifest.resource_limits.max_input_bytes,
            "max_output_bytes": manifest.resource_limits.max_output_bytes,
            "max_rows": manifest.resource_limits.max_rows,
            "timeout_seconds": manifest.resource_limits.timeout_seconds,
            "max_memory_mb": manifest.resource_limits.max_memory_mb,
            "package_install_enabled": manifest.resource_limits.package_install_enabled,
        },
        "os_sandbox": os_sandbox or {"applied": False, "errors": ["not_requested"]},
        "inputs": [{"path": str(input_path), "content_hash": input_hash}],
        "artifacts": artifact_records,
        "data_lineage": {
            "input_count": 1,
            "artifact_count": len(artifact_records),
            "edges": [
                {
                    "from_input_hash": input_hash,
                    "to_artifact_hash": record["content_hash"],
                    "artifact_kind": record["artifact_kind"],
                }
                for record in artifact_records
            ],
        },
    }
    manifest_path = output_dir / "research_job_manifest.json"
    manifest_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
    return manifest_path


def apply_runtime_sandbox_limits() -> dict[str, object]:
    """Best-effort process limits for local-only research jobs.

    Rust owns policy approval before launching this worker. Python applies OS
    limits in the child where available and records the result in the manifest.
    """

    requested_cpu = int(os.environ.get("RESEARCHCODE_WORKER_TIMEOUT_SECONDS", "30"))
    requested_memory_mb = int(os.environ.get("RESEARCHCODE_WORKER_MAX_MEMORY_MB", "512"))
    report: dict[str, object] = {
        "applied": False,
        "cpu_seconds": requested_cpu,
        "max_memory_mb": requested_memory_mb,
        "errors": [],
    }
    try:
        import resource  # type: ignore[import-not-found]
    except Exception as error:  # pragma: no cover - platform dependent.
        report["errors"] = [f"resource_unavailable:{type(error).__name__}"]
        return report

    errors: list[str] = []
    applied_any = False
    try:
        resource.setrlimit(resource.RLIMIT_CPU, (requested_cpu, requested_cpu))
        applied_any = True
    except Exception as error:  # pragma: no cover - platform dependent.
        errors.append(f"cpu_limit_failed:{type(error).__name__}")
    if hasattr(resource, "RLIMIT_AS"):
        try:
            max_bytes = requested_memory_mb * 1024 * 1024
            resource.setrlimit(resource.RLIMIT_AS, (max_bytes, max_bytes))
            applied_any = True
        except Exception as error:  # pragma: no cover - macOS can reject some limits.
            errors.append(f"memory_limit_failed:{type(error).__name__}")
    else:  # pragma: no cover - platform dependent.
        errors.append("memory_limit_unavailable")
    report["applied"] = applied_any
    report["errors"] = errors
    return report


def profile_csv_command(args: list[str]) -> int:
    if len(args) != 3:
        print("usage: python -m research_worker profile-csv <job_id> <input.csv> <output_dir>", file=sys.stderr)
        return 2
    job_id = args[0]
    input_path = Path(args[1])
    output_dir = Path(args[2])
    os_sandbox = apply_runtime_sandbox_limits()
    profile_path = write_profile(input_path, output_dir)
    privacy_report_path = write_privacy_report(profile_path, output_dir)
    analysis_script_path = write_analysis_script(profile_path, output_dir)
    report_path = write_markdown_report(profile_path, privacy_report_path, output_dir)
    notebook_path = write_notebook(profile_path, report_path, output_dir)
    artifacts = [
        ("data_profile", profile_path),
        ("privacy_report", privacy_report_path),
        ("analysis_script", analysis_script_path),
        ("markdown_report", report_path),
        ("notebook", notebook_path),
    ]
    manifest_path = write_job_manifest(job_id, input_path, artifacts, output_dir, os_sandbox=os_sandbox)
    print(
        json.dumps(
            {
                "profile": str(profile_path),
                "privacy_report": str(privacy_report_path),
                "analysis_script": str(analysis_script_path),
                "report": str(report_path),
                "notebook": str(notebook_path),
                "manifest": str(manifest_path),
            },
            sort_keys=True,
        )
    )
    return 0


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: python -m research_worker <command> ...", file=sys.stderr)
        return 2
    command = sys.argv[1]
    if command == "profile-csv":
        return profile_csv_command(sys.argv[2:])
    print(f"unknown command: {command}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
