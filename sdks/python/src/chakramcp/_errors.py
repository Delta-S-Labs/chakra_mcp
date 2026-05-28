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


class QuotaExhaustedError(ChakraMCPError):
    """Raised by `invoke` (public mode) when the per-invoker monthly
    quota on the target capability has been exhausted (HTTP 429, body
    code ``monthly_quota_exhausted``, migration 0022). Carries the
    quota the owner set and the UTC instant the window resets so
    callers can back off intelligently."""

    def __init__(self, message: str, quota: int, resets_at: str) -> None:
        super().__init__(429, "monthly_quota_exhausted", message)
        self.quota = quota
        self.resets_at = resets_at


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
        # Public-invoke monthly quota carries extra structured fields;
        # surface it as a typed subclass so callers don't have to parse
        # the envelope themselves.
        if (
            response.status_code == 429
            and err.get("code") == "monthly_quota_exhausted"
            and isinstance(body.get("quota"), int)
            and isinstance(body.get("resets_at"), str)
        ):
            raise QuotaExhaustedError(
                err.get("message", response.reason_phrase),
                body["quota"],
                body["resets_at"],
            )
        raise ChakraMCPError(
            response.status_code,
            err.get("code", "unknown"),
            err.get("message", response.reason_phrase),
        )
    raise ChakraMCPError(response.status_code, "unknown", response.text or response.reason_phrase)
