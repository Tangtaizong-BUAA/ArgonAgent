"""Research job manifest sketch without external dependencies."""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class ResearchWorkerLimits:
    max_input_bytes: int = 10_000_000
    max_output_bytes: int = 20_000_000
    max_rows: int = 100_000
    timeout_seconds: int = 30
    max_memory_mb: int = 512
    package_install_enabled: bool = False


@dataclass(frozen=True)
class ResearchJobManifest:
    job_id: str
    input_paths: tuple[str, ...]
    output_dir: str
    network_enabled: bool = False
    privacy_class: str = "internal"
    resource_limits: ResearchWorkerLimits = field(default_factory=ResearchWorkerLimits)

    def validate(self) -> None:
        if self.network_enabled:
            raise ValueError("network must be disabled by default")
        if self.resource_limits.package_install_enabled:
            raise ValueError("package install must be disabled by default")
        if not self.input_paths:
            raise ValueError("at least one input path is required")
        if self.resource_limits.max_input_bytes <= 0:
            raise ValueError("max_input_bytes must be positive")
        if self.resource_limits.max_output_bytes <= 0:
            raise ValueError("max_output_bytes must be positive")
        if self.resource_limits.max_rows <= 0:
            raise ValueError("max_rows must be positive")
        if self.resource_limits.timeout_seconds <= 0:
            raise ValueError("timeout_seconds must be positive")
        if self.resource_limits.max_memory_mb <= 0:
            raise ValueError("max_memory_mb must be positive")
