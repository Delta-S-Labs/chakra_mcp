"""Error envelope decoding."""

from __future__ import annotations

from typing import Any

import httpx


class ChakraMCPError(Exception):
    """Raised when an API call returns a non-2xx status."""

    def __init__(self, status: int, code: str, message: str) -> None:
        super().__init__(f"[{status} {code}] {message}")
        self.status = status
        self.code = code
        self.message = message


def raise_for_response(response: httpx.Response) -> None:
    """Decode the standard `{"error": {"code", "message"}}` envelope."""
    if response.is_success:
        return
    body: Any
    try:
        body = response.json()
    except Exception:
        body = None
    if isinstance(body, dict) and isinstance(body.get("error"), dict):
        err = body["error"]
        raise ChakraMCPError(
            response.status_code,
            err.get("code", "unknown"),
            err.get("message", response.reason_phrase),
        )
    raise ChakraMCPError(response.status_code, "unknown", response.text or response.reason_phrase)
