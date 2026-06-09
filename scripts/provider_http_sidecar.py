#!/usr/bin/env python3
"""Disabled-by-default provider HTTP sidecar.

The Rust runtime owns model preflight, safety gates, event recording, stream
parsing, and transcript sanitization. This sidecar is intentionally narrow: it
turns an already prepared HTTP request into bytes only when network access is
explicitly enabled with RESEARCHCODE_ALLOW_NETWORK=1.

Input is a JSON object on stdin:
  method, url, authorization_env, body_json, stream, response_body_path

Set mode=health_check to validate provider/network/auth wiring without writing
the provider response body. Health checks are still disabled unless
RESEARCHCODE_ALLOW_NETWORK=1.

The API key is read from the named environment variable only. The key value is
never printed or written to the response body file.
"""

from __future__ import annotations

import json
import os
import re
import socket
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


ANTHROPIC_VERSION = "2023-06-01"


def main() -> int:
    try:
        request = json.loads(sys.stdin.read())
        if request.get("mode") == "health_check":
            result = health_check_prepared_request(request)
        elif request.get("mode") == "stream_visible_text":
            stream_visible_text_prepared_request(request)
            return 0
        else:
            result = send_prepared_request(request)
    except SidecarError as error:
        print(json.dumps({"ok": False, "error": error.code}, sort_keys=True), file=sys.stderr)
        return 2
    except Exception as error:  # pragma: no cover - defensive outer boundary.
        print(json.dumps({"ok": False, "error": type(error).__name__}, sort_keys=True), file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


class SidecarError(Exception):
    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


def send_prepared_request(request: dict[str, Any]) -> dict[str, Any]:
    method, url, authorization_env, body_json, parsed_body, target_url = validate_prepared_request(request)
    response_body_path = Path(require_string(request, "response_body_path"))
    stream = bool(request.get("stream", False))

    api_key = read_api_key_or_skip(authorization_env)
    if isinstance(api_key, dict):
        return api_key

    headers = build_headers(target_url, api_key)
    http_request = urllib.request.Request(
        target_url,
        data=body_json.encode("utf-8"),
        method="POST",
        headers=headers,
    )
    try:
        with urllib.request.urlopen(http_request, timeout=http_timeout(60)) as response:
            body = response.read()
            status_code = response.status
    except urllib.error.HTTPError as error:
        body = error.read()
        status_code = error.code
    response_body_path.parent.mkdir(parents=True, exist_ok=True)
    response_body_path.write_bytes(body)
    return {
        "ok": True,
        "skipped": False,
        "status_code": status_code,
        "stream": stream,
        "bytes_written": len(body),
    }


def health_check_prepared_request(request: dict[str, Any]) -> dict[str, Any]:
    _, _, authorization_env, body_json, parsed_body, target_url = validate_prepared_request(request)
    api_key = read_api_key_or_skip(authorization_env)
    if isinstance(api_key, dict):
        return {
            "ok": True,
            "health_status": "skipped",
            "skipped": True,
            "reason": api_key["reason"],
        }
    headers = build_headers(target_url, api_key)
    http_request = urllib.request.Request(
        target_url,
        data=body_json.encode("utf-8"),
        method="POST",
        headers=headers,
    )
    try:
        with urllib.request.urlopen(http_request, timeout=http_timeout(30)) as response:
            response.read(1024)
            status_code = response.status
    except urllib.error.HTTPError as error:
        error.read(1024)
        status_code = error.code
    return {
        "ok": True,
        "health_status": "healthy" if 200 <= status_code < 300 else "unhealthy",
        "skipped": False,
        "status_code": status_code,
        "target_kind": target_kind(target_url, parsed_body),
    }


def stream_visible_text_prepared_request(request: dict[str, Any]) -> None:
    """Stream sanitized visible text deltas as JSONL.

    This mode exists for terminal/TUI UX only. It deliberately does not replay
    DeepSeek thinking text as visible output, because DeepSeek's Anthropic SSE
    stream carries thinking deltas separately from normal text deltas.
    """

    _, _, authorization_env, body_json, parsed_body, target_url = validate_prepared_request(request)
    response_body_file = None
    response_body_path_value = request.get("response_body_path")
    if isinstance(response_body_path_value, str) and response_body_path_value and response_body_path_value != "/dev/null":
        response_body_path = Path(response_body_path_value)
        response_body_path.parent.mkdir(parents=True, exist_ok=True)
        response_body_file = response_body_path.open("wb")
    api_key = read_api_key_or_skip(authorization_env)
    if isinstance(api_key, dict):
        if response_body_file is not None:
            response_body_file.close()
        emit_jsonl({"event": "skipped", "reason": api_key["reason"]})
        emit_jsonl({"event": "done"})
        return

    headers = build_headers(target_url, api_key)
    http_request = urllib.request.Request(
        target_url,
        data=body_json.encode("utf-8"),
        method="POST",
        headers=headers,
    )
    try:
        with urllib.request.urlopen(http_request, timeout=http_timeout(60)) as response:
            emit_jsonl(
                {
                    "event": "http_status",
                    "status_code": response.status,
                    "target_kind": target_kind(target_url, parsed_body),
                }
            )
            content_block_types: dict[str, str] = {}
            for raw_line in response:
                if response_body_file is not None:
                    response_body_file.write(raw_line)
                    response_body_file.flush()
                handle_stream_visible_line(
                    raw_line.decode("utf-8", errors="replace"),
                    content_block_types,
                )
    except urllib.error.HTTPError as error:
        body = error.read()
        if response_body_file is not None:
            response_body_file.write(body)
            response_body_file.flush()
        emit_jsonl(
            {
                "event": "http_error",
                "status_code": error.code,
                "preview": sanitize_visible_delta(body.decode("utf-8", errors="replace"))[:1200],
            }
        )
        return
    except (TimeoutError, socket.timeout) as error:
        emit_jsonl(
            {
                "event": "transport_error",
                "preview": sanitize_visible_delta(f"timeout: {error}")[:1200],
            }
        )
        return
    except urllib.error.URLError as error:
        emit_jsonl(
            {
                "event": "transport_error",
                "preview": sanitize_visible_delta(str(error))[:1200],
            }
        )
        return
    finally:
        if response_body_file is not None:
            response_body_file.close()
    emit_jsonl({"event": "done"})


def handle_stream_visible_line(line: str, content_block_types: dict[str, str] | None = None) -> None:
    if content_block_types is None:
        content_block_types = {}
    stripped = line.strip()
    if not stripped or stripped.startswith(":") or stripped.startswith("event:"):
        return
    if stripped.startswith("data:"):
        payload = stripped[len("data:") :].strip()
    elif stripped.startswith("{"):
        payload = stripped
    else:
        return
    if payload == "[DONE]":
        emit_jsonl({"event": "done"})
        return
    try:
        data = json.loads(payload)
    except json.JSONDecodeError:
        emit_jsonl({"event": "parse_warning", "reason": "invalid_json_payload"})
        return

    event_type = data.get("type")
    if event_type == "content_block_delta":
        delta = data.get("delta")
        if isinstance(delta, dict):
            delta_type = delta.get("type")
            if delta_type == "text_delta":
                text = delta.get("text", "")
                if text:
                    emit_jsonl({"event": "text", "delta": sanitize_visible_delta(str(text))})
            elif delta_type == "thinking_delta":
                thinking = delta.get("thinking", "")
                if thinking:
                    emit_jsonl(
                        {
                            "event": "reasoning_passthrough_delta",
                            "delta_hex": str(thinking).encode("utf-8").hex(),
                        }
                    )
                emit_jsonl(
                    {
                        "event": "reasoning_sanitized",
                        "chars": len(sanitize_visible_delta(str(thinking))),
                    }
                )
            elif delta_type == "signature_delta":
                signature = delta.get("signature", "")
                if signature:
                    emit_jsonl(
                        {
                            "event": "reasoning_signature_delta",
                            "delta_hex": str(signature).encode("utf-8").hex(),
                        }
                    )
            elif delta_type == "input_json_delta":
                partial_json = delta.get("partial_json", "")
                emit_jsonl(
                    {
                        "event": "tool_arguments_delta",
                        "chars": len(str(partial_json)),
                        # Tool arguments are machine input, not visible model text.
                        # Keep the executable bytes exact for the Rust runtime while
                        # avoiding accidental terminal display of raw argument text.
                        "delta_hex": str(partial_json).encode("utf-8").hex(),
                        "delta_preview": sanitize_visible_delta(str(partial_json))[:80],
                    }
                )
        return
    if event_type == "content_block_start":
        content_block = data.get("content_block")
        index = data.get("index")
        block_key = str(index) if index is not None else "_current"
        if isinstance(content_block, dict):
            block_type = str(content_block.get("type", "unknown"))
            content_block_types[block_key] = block_type
            emit_jsonl({"event": "content_block_start", "index": index, "block_type": block_type})
        if isinstance(content_block, dict) and content_block.get("type") == "tool_use":
            input_json = content_block.get("input")
            emit_jsonl(
                {
                    "event": "tool_call",
                    "id": str(content_block.get("id", "")),
                    "name": str(content_block.get("name", "")),
                }
            )
            if input_json not in (None, {}, ""):
                input_payload = json.dumps(input_json, ensure_ascii=False, sort_keys=True)
                emit_jsonl(
                    {
                        "event": "tool_arguments_delta",
                        "chars": len(input_payload),
                        "delta_hex": input_payload.encode("utf-8").hex(),
                        "delta_preview": sanitize_visible_delta(input_payload)[:80],
                    }
                )
        return
    if event_type == "message_delta":
        delta = data.get("delta")
        if isinstance(delta, dict):
            stop_reason = delta.get("stop_reason")
            if stop_reason:
                emit_jsonl({"event": "stop_reason", "stop_reason": str(stop_reason)})
        usage = data.get("usage")
        if isinstance(usage, dict):
            emit_jsonl(
                {
                    "event": "usage",
                    "input_tokens": usage.get("input_tokens"),
                    "output_tokens": usage.get("output_tokens"),
                    "cache_read_input_tokens": usage.get("cache_read_input_tokens"),
                    "cache_creation_input_tokens": usage.get("cache_creation_input_tokens"),
                }
            )
        return
    if event_type == "content_block_stop":
        index = data.get("index")
        block_key = str(index) if index is not None else "_current"
        block_type = content_block_types.pop(block_key, "unknown")
        emit_jsonl({"event": "content_block_stop", "index": index, "block_type": block_type})
        return
    if event_type == "message_stop":
        emit_jsonl({"event": "done"})
        return

    # Anthropic Messages non-SSE fallback. Some compatible endpoints can return
    # a final JSON message even when the request asked for stream=true; the TUI
    # still needs visible text rather than a silent token-only completion.
    content = data.get("content")
    if isinstance(content, list):
        for block in content:
            if isinstance(block, dict) and block.get("type") == "text":
                text = block.get("text", "")
                if text:
                    emit_jsonl({"event": "text", "delta": sanitize_visible_delta(str(text))})
            if isinstance(block, dict) and block.get("type") == "tool_use":
                input_json = block.get("input")
                emit_jsonl(
                    {
                        "event": "tool_call",
                        "id": str(block.get("id", "")),
                        "name": str(block.get("name", "")),
                    }
                )
                if input_json not in (None, {}, ""):
                    input_payload = json.dumps(input_json, ensure_ascii=False, sort_keys=True)
                    emit_jsonl(
                        {
                            "event": "tool_arguments_delta",
                            "chars": len(input_payload),
                            "delta_hex": input_payload.encode("utf-8").hex(),
                            "delta_preview": sanitize_visible_delta(input_payload)[:80],
                        }
                    )
        usage = data.get("usage")
        if isinstance(usage, dict):
            emit_jsonl(
                {
                    "event": "usage",
                    "input_tokens": usage.get("input_tokens"),
                    "output_tokens": usage.get("output_tokens"),
                    "cache_read_input_tokens": usage.get("cache_read_input_tokens"),
                    "cache_creation_input_tokens": usage.get("cache_creation_input_tokens"),
                }
            )
        return

    # OpenAI-compatible fallback for providers that stream choices[].delta.
    choices = data.get("choices")
    usage = data.get("usage")
    if isinstance(usage, dict):
        completion_details = usage.get("completion_tokens_details")
        reasoning_tokens = None
        if isinstance(completion_details, dict):
            reasoning_tokens = completion_details.get("reasoning_tokens")
        emit_jsonl(
            {
                "event": "usage",
                "input_tokens": usage.get("prompt_tokens") or usage.get("input_tokens"),
                "output_tokens": usage.get("completion_tokens") or usage.get("output_tokens"),
                "reasoning_tokens": usage.get("reasoning_tokens") or reasoning_tokens,
                "cache_read_input_tokens": usage.get("prompt_cache_hit_tokens")
                or usage.get("cache_read_input_tokens"),
                "cache_creation_input_tokens": usage.get("prompt_cache_miss_tokens")
                or usage.get("cache_creation_input_tokens"),
            }
        )
    if isinstance(choices, list) and choices:
        delta = choices[0].get("delta") if isinstance(choices[0], dict) else None
        if isinstance(delta, dict):
            reasoning = delta.get("reasoning_content") or delta.get("thinking") or delta.get("reasoning")
            if reasoning:
                emit_jsonl(
                    {
                        "event": "reasoning_passthrough_delta",
                        "delta_hex": str(reasoning).encode("utf-8").hex(),
                    }
                )
                emit_jsonl(
                    {
                        "event": "reasoning_sanitized",
                        "chars": len(sanitize_visible_delta(str(reasoning))),
                    }
                )
            text = delta.get("content")
            if text:
                emit_jsonl({"event": "text", "delta": sanitize_visible_delta(str(text))})
            tool_calls = delta.get("tool_calls")
            if isinstance(tool_calls, list):
                for call in tool_calls:
                    if not isinstance(call, dict):
                        continue
                    function = call.get("function")
                    if not isinstance(function, dict):
                        continue
                    name = str(function.get("name", ""))
                    arguments = str(function.get("arguments", ""))
                    call_id = str(call.get("id", ""))
                    if name or call_id:
                        emit_jsonl({"event": "tool_call", "id": call_id, "name": name})
                    if arguments:
                        emit_jsonl(
                            {
                                "event": "tool_arguments_delta",
                                "chars": len(arguments),
                                "delta_hex": arguments.encode("utf-8").hex(),
                                "delta_preview": sanitize_visible_delta(arguments)[:80],
                            }
                        )


def emit_jsonl(payload: dict[str, Any]) -> None:
    print(json.dumps(payload, ensure_ascii=False, sort_keys=True), flush=True)


def validate_prepared_request(request: dict[str, Any]) -> tuple[str, str, str, str, dict[str, Any], str]:
    method = require_string(request, "method")
    url = require_string(request, "url")
    authorization_env = require_string(request, "authorization_env")
    body_json = require_string(request, "body_json")
    if method.upper() != "POST":
        raise SidecarError("unsupported_method")
    if looks_like_secret(authorization_env) or not re.fullmatch(r"[A-Z0-9_]+", authorization_env):
        raise SidecarError("invalid_authorization_env")
    parsed_body = json.loads(body_json)
    if not isinstance(parsed_body, dict):
        raise SidecarError("body_json_must_be_object")
    target_url = normalized_target_url(url, parsed_body)
    return method, url, authorization_env, body_json, parsed_body, target_url


def read_api_key_or_skip(authorization_env: str) -> str | dict[str, Any]:
    if os.environ.get("RESEARCHCODE_ALLOW_NETWORK") != "1":
        return {"ok": True, "skipped": True, "reason": "network_not_enabled"}
    api_key = os.environ.get(authorization_env, "")
    if not api_key:
        return {"ok": True, "skipped": True, "reason": "missing_api_key"}
    return api_key


def http_timeout(default_seconds: int) -> float:
    raw = os.environ.get("RESEARCHCODE_PROVIDER_HTTP_TIMEOUT_SECONDS", "").strip()
    if not raw:
        return float(default_seconds)
    try:
        parsed = float(raw)
    except ValueError:
        return float(default_seconds)
    return min(max(parsed, 0.5), float(default_seconds))


def require_string(request: dict[str, Any], key: str) -> str:
    value = request.get(key)
    if not isinstance(value, str) or not value:
        raise SidecarError(f"missing_{key}")
    return value


def normalized_target_url(url: str, body: dict[str, Any]) -> str:
    trimmed = url.rstrip("/")
    model = str(body.get("model", ""))
    if trimmed.endswith("/anthropic"):
        return f"{trimmed}/v1/messages"
    if model.startswith("deepseek-"):
        if trimmed.endswith("/v1"):
            return f"{trimmed}/chat/completions"
        if trimmed == "https://api.deepseek.com":
            return f"{trimmed}/chat/completions"
    if trimmed.endswith("/v1") and model.startswith("Qwen/"):
        return f"{trimmed}/chat/completions"
    return trimmed


def build_headers(target_url: str, api_key: str) -> dict[str, str]:
    headers = {"content-type": "application/json"}
    if "/anthropic/" in target_url or target_url.endswith("/v1/messages"):
        headers["x-api-key"] = api_key
        headers["anthropic-version"] = ANTHROPIC_VERSION
    else:
        headers["authorization"] = f"Bearer {api_key}"
    return headers


def target_kind(target_url: str, body: dict[str, Any]) -> str:
    model = str(body.get("model", ""))
    if "/anthropic/" in target_url or target_url.endswith("/v1/messages"):
        return "anthropic_messages"
    if target_url.endswith("/chat/completions") or model.startswith("Qwen/"):
        return "openai_chat_completions"
    return "custom_post"


def looks_like_secret(value: str) -> bool:
    stripped = value.strip()
    return stripped.startswith("sk-") or stripped.startswith("AKIA") or len(stripped) > 80


def sanitize_visible_delta(value: str) -> str:
    value = re.sub(r"sk-[A-Za-z0-9_-]+", "[REDACTED_SECRET]", value)
    value = value.replace(".env", "[REDACTED_PATH]")
    value = value.replace("id_rsa", "[REDACTED_PATH]")
    return value


if __name__ == "__main__":
    raise SystemExit(main())
