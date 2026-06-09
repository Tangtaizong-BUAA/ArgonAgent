#!/usr/bin/env python3
"""Optional live DeepSeek Anthropic-compatible API smoke.

Default behavior is safe: if RESEARCHCODE_ALLOW_NETWORK=1 and DEEPSEEK_API_KEY
are not both present, the script exits successfully with a skipped result.

When enabled, it makes one minimal non-streaming request to the Anthropic-style
messages endpoint. It never prints, stores, or echoes the API key.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request


def main() -> int:
    if os.environ.get("RESEARCHCODE_ALLOW_NETWORK") != "1":
        print(json.dumps({"ok": True, "skipped": True, "reason": "network_not_enabled"}, sort_keys=True))
        return 0
    api_key = os.environ.get("DEEPSEEK_API_KEY", "")
    if not api_key:
        print(json.dumps({"ok": True, "skipped": True, "reason": "missing_deepseek_api_key"}, sort_keys=True))
        return 0
    base_url = os.environ.get("DEEPSEEK_ANTHROPIC_BASE_URL", "https://api.deepseek.com/anthropic")
    model = os.environ.get("DEEPSEEK_MODEL", "deepseek-v4-flash")
    url = base_url.rstrip("/") + "/v1/messages"
    body = {
        "model": model,
        "max_tokens": 32,
        "system": "You are a concise API smoke-test assistant.",
        "messages": [
            {
                "role": "user",
                "content": [{"type": "text", "text": "Reply with OK."}],
            }
        ],
        "stream": False,
    }
    request = urllib.request.Request(
        url,
        data=json.dumps(body).encode("utf-8"),
        method="POST",
        headers={
            "content-type": "application/json",
            "x-api-key": api_key,
            "anthropic-version": "2023-06-01",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")[:500]
        print(json.dumps({"ok": False, "status": error.code, "error": detail}, sort_keys=True), file=sys.stderr)
        return 1
    except OSError as error:
        print(json.dumps({"ok": False, "error": str(error)}, sort_keys=True), file=sys.stderr)
        return 1
    content = payload.get("content", [])
    text = ""
    if content and isinstance(content[0], dict):
        text = str(content[0].get("text", ""))
    print(
        json.dumps(
            {
                "ok": True,
                "skipped": False,
                "model": payload.get("model", model),
                "content_preview": text[:80],
                "has_usage": "usage" in payload,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
