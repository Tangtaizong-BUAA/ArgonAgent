from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path

from scripts.provider_http_sidecar import handle_stream_visible_line, normalized_target_url


ROOT = Path(__file__).resolve().parents[1]
SIDECAR = ROOT / "scripts" / "provider_http_sidecar.py"


class ProviderHttpSidecarTests(unittest.TestCase):
    def test_default_is_network_disabled_skip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            request = self.request(Path(tmp) / "body.txt")
            env = os.environ.copy()
            env.pop("RESEARCHCODE_ALLOW_NETWORK", None)
            completed = subprocess.run(
                ["python3", str(SIDECAR)],
                input=json.dumps(request),
                text=True,
                capture_output=True,
                env=env,
                check=False,
            )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        payload = json.loads(completed.stdout)
        self.assertTrue(payload["skipped"])
        self.assertEqual(payload["reason"], "network_not_enabled")
        self.assertNotIn("sk-", completed.stdout + completed.stderr)

    def test_rejects_raw_key_as_authorization_env(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            request = self.request(Path(tmp) / "body.txt")
            request["authorization_env"] = "sk-testsecret"
            completed = subprocess.run(
                ["python3", str(SIDECAR)],
                input=json.dumps(request),
                text=True,
                capture_output=True,
                check=False,
            )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("invalid_authorization_env", completed.stderr)
        self.assertNotIn("sk-testsecret", completed.stdout + completed.stderr)

    def test_normalizes_deepseek_anthropic_base_url(self) -> None:
        target = normalized_target_url(
            "https://api.deepseek.com/anthropic",
            {"model": "deepseek-v4-flash"},
        )
        self.assertEqual(target, "https://api.deepseek.com/anthropic/v1/messages")

    def test_normalizes_deepseek_openai_base_url(self) -> None:
        target = normalized_target_url(
            "https://api.deepseek.com",
            {"model": "deepseek-v4-flash"},
        )
        self.assertEqual(target, "https://api.deepseek.com/chat/completions")

    def test_health_check_is_disabled_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            request = self.request(Path(tmp) / "body.txt")
            request["mode"] = "health_check"
            env = os.environ.copy()
            env.pop("RESEARCHCODE_ALLOW_NETWORK", None)
            completed = subprocess.run(
                ["python3", str(SIDECAR)],
                input=json.dumps(request),
                text=True,
                capture_output=True,
                env=env,
                check=False,
            )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        payload = json.loads(completed.stdout)
        self.assertEqual(payload["health_status"], "skipped")
        self.assertTrue(payload["skipped"])
        self.assertEqual(payload["reason"], "network_not_enabled")
        self.assertNotIn("sk-", completed.stdout + completed.stderr)

    def test_stream_visible_line_keeps_thinking_out_of_visible_text(self) -> None:
        output = StringIO()
        with redirect_stdout(output):
            handle_stream_visible_line(
                'data: {"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"Need sk-testsecret from .env"}}'
            )
            handle_stream_visible_line(
                'data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Visible OK"}}'
            )
            handle_stream_visible_line(
                'data: {"type":"message_delta","usage":{"input_tokens":11,"output_tokens":7,"cache_read_input_tokens":5}}'
            )
        lines = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(lines[0]["event"], "reasoning_passthrough_delta")
        self.assertIn("delta_hex", lines[0])
        self.assertEqual(lines[1]["event"], "reasoning_sanitized")
        self.assertEqual(lines[2], {"delta": "Visible OK", "event": "text"})
        self.assertEqual(lines[3]["input_tokens"], 11)
        serialized = output.getvalue()
        self.assertNotIn("sk-testsecret", serialized)
        self.assertNotIn(".env", serialized)

    def test_stream_visible_line_emits_tool_id_and_argument_delta(self) -> None:
        output = StringIO()
        with redirect_stdout(output):
            handle_stream_visible_line(
                'data: {"type":"content_block_start","content_block":{"type":"tool_use","id":"toolu_1","name":"file_read"}}'
            )
            handle_stream_visible_line(
                'data: {"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{\\"path\\":\\"README.md\\"}"}}'
            )
        lines = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(lines[0]["event"], "content_block_start")
        self.assertEqual(lines[0]["block_type"], "tool_use")
        self.assertEqual(lines[1], {"event": "tool_call", "id": "toolu_1", "name": "file_read"})
        self.assertEqual(lines[2]["event"], "tool_arguments_delta")
        self.assertNotIn("delta", lines[2])
        self.assertEqual(bytes.fromhex(lines[2]["delta_hex"]).decode("utf-8"), '{"path":"README.md"}')
        self.assertIn("delta_preview", lines[2])
        self.assertEqual(lines[2]["chars"], len('{"path":"README.md"}'))

    def test_stream_visible_line_preserves_tool_input_from_start_block(self) -> None:
        output = StringIO()
        with redirect_stdout(output):
            handle_stream_visible_line(
                'data: {"type":"content_block_start","content_block":{"type":"tool_use","id":"toolu_1","name":"file_write","input":{"content":"<html>OK</html>","path":"demo.html"}}}'
            )
        lines = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(lines[0]["event"], "content_block_start")
        self.assertEqual(lines[0]["block_type"], "tool_use")
        self.assertEqual(lines[1], {"event": "tool_call", "id": "toolu_1", "name": "file_write"})
        self.assertEqual(lines[2]["event"], "tool_arguments_delta")
        decoded = bytes.fromhex(lines[2]["delta_hex"]).decode("utf-8")
        self.assertEqual(json.loads(decoded), {"content": "<html>OK</html>", "path": "demo.html"})
        self.assertNotIn("delta", lines[2])

    def test_stream_visible_line_tags_content_block_stop_by_type(self) -> None:
        output = StringIO()
        block_types: dict[str, str] = {}
        with redirect_stdout(output):
            handle_stream_visible_line(
                'data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}',
                block_types,
            )
            handle_stream_visible_line(
                'data: {"type":"content_block_stop","index":1}',
                block_types,
            )
        lines = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(lines[0]["event"], "content_block_start")
        self.assertEqual(lines[0]["block_type"], "text")
        self.assertEqual(lines[1], {"event": "content_block_stop", "index": 1, "block_type": "text"})

    def test_stream_visible_line_emits_message_stop_reason(self) -> None:
        output = StringIO()
        with redirect_stdout(output):
            handle_stream_visible_line(
                'data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"input_tokens":11,"output_tokens":7}}'
            )
        lines = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(lines[0], {"event": "stop_reason", "stop_reason": "max_tokens"})
        self.assertEqual(lines[1]["event"], "usage")

    def test_stream_visible_line_preserves_tool_input_from_final_json_message(self) -> None:
        output = StringIO()
        with redirect_stdout(output):
            handle_stream_visible_line(
                '{"content":[{"type":"tool_use","id":"toolu_2","name":"file_write","input":{"path":"demo.html","content":"<html>OK</html>"}}],"usage":{"input_tokens":4,"output_tokens":5}}'
            )
        lines = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(lines[0], {"event": "tool_call", "id": "toolu_2", "name": "file_write"})
        self.assertEqual(lines[1]["event"], "tool_arguments_delta")
        decoded = bytes.fromhex(lines[1]["delta_hex"]).decode("utf-8")
        self.assertEqual(json.loads(decoded), {"content": "<html>OK</html>", "path": "demo.html"})
        self.assertEqual(lines[2]["event"], "usage")

    def test_stream_visible_line_emits_openai_tool_call(self) -> None:
        output = StringIO()
        with redirect_stdout(output):
            handle_stream_visible_line(
                'data: {"choices":[{"delta":{"tool_calls":[{"id":"call_1","function":{"name":"file_read","arguments":"{\\"path\\":\\"README.md\\"}"}}]}}]}'
            )
        lines = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(lines[0], {"event": "tool_call", "id": "call_1", "name": "file_read"})
        self.assertEqual(lines[1]["event"], "tool_arguments_delta")
        self.assertEqual(
            bytes.fromhex(lines[1]["delta_hex"]).decode("utf-8"),
            '{"path":"README.md"}',
        )

    def request(self, response_path: Path) -> dict[str, object]:
        return {
            "method": "POST",
            "url": "https://api.deepseek.com/anthropic",
            "authorization_env": "DEEPSEEK_API_KEY",
            "body_json": json.dumps(
                {
                    "model": "deepseek-v4-flash",
                    "max_tokens": 16,
                    "stream": False,
                    "messages": [{"role": "user", "content": "Reply OK"}],
                }
            ),
            "stream": False,
            "response_body_path": str(response_path),
        }


if __name__ == "__main__":
    unittest.main()
