"""Error envelope decoding."""

from __future__ import annotations

from typing import Any

import httpx


class ChakraMCPError(Exception):
    """Raised when an API call returns a non-2xx status.

    ``retry_after_secs`` is how long to wait before retrying, when the
    server says: the ``Retry-After`` header, else the body's
    ``retry_after_secs``. ``None`` when it says neither (most errors,
    and older servers)."""

    def __init__(
        self,
        status: int,
        code: str,
        message: str,
        retry_after_secs: int | None = None,
    ) -> None:
        super().__init__(f"[{status} {code}] {message}")
        self.status = status
        self.code = code
        self.message = message
        self.retry_after_secs = retry_after_secs


class QuotaExhaustedError(ChakraMCPError):
    """Raised by `invoke` (public mode) when the per-invoker monthly
    quota on the target capability has been exhausted (HTTP 429, body
    code ``monthly_quota_exhausted``, migration 0022). Carries the
    quota the owner set and the UTC instant the window resets so
    callers can back off intelligently."""

    def __init__(
        self,
        message: str,
        quota: int,
        resets_at: str,
        retry_after_secs: int | None = None,
    ) -> None:
        super().__init__(429, "monthly_quota_exhausted", message, retry_after_secs)
        self.quota = quota
        self.resets_at = resets_at


class RateLimitedError(ChakraMCPError):
    """Raised when the calling account is over its per-minute rate
    limit (HTTP 429, body code ``account_rate_limited``).
    ``retry_after_secs`` is the rest of the minute. The SDK doesn't
    retry on its own."""

    def __init__(self, message: str, retry_after_secs: int | None = None) -> None:
        super().__init__(429, "account_rate_limited", message, retry_after_secs)


class CreditsExhaustedError(ChakraMCPError):
    """Raised when the calling account is out of credits (HTTP 429,
    body code ``account_credits_exhausted``). ``retry_after_secs`` runs
    until the next free monthly grant (``next_grant_at``, a UTC
    instant) and can be weeks long; buying credits works at once.
    ``credits_url`` is where to buy them, and is ``None`` on
    self-hosted servers. All three are ``None`` on older servers."""

    def __init__(
        self,
        message: str,
        retry_after_secs: int | None = None,
        next_grant_at: str | None = None,
        credits_url: str | None = None,
    ) -> None:
        super().__init__(429, "account_credits_exhausted", message, retry_after_secs)
        self.next_grant_at = next_grant_at
        self.credits_url = credits_url


def _retry_after_secs(response: httpx.Response, err: dict[str, Any]) -> int | None:
    """The ``Retry-After`` header when it's a whole number of seconds,
    else the body's ``retry_after_secs``, else ``None``."""
    header = response.headers.get("retry-after", "").strip()
    if header.isascii() and header.isdigit():
        return int(header)
    body = err.get("retry_after_secs")
    # bool is an int subclass; a JSON true is not a delay.
    if isinstance(body, int) and not isinstance(body, bool) and body >= 0:
        return body
    return None


def _str_or_none(value: Any) -> str | None:
    return value if isinstance(value, str) else None


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
        code = err.get("code", "unknown")
        message = err.get("message", response.reason_phrase)
        retry_after = _retry_after_secs(response, err)
        # Usage limits (429s) carry structured fields; surface them as
        # typed subclasses so callers don't have to parse the envelope
        # themselves.
        if response.status_code == 429:
            # Public-invoke monthly quota on the target capability.
            if (
                code == "monthly_quota_exhausted"
                and isinstance(body.get("quota"), int)
                and isinstance(body.get("resets_at"), str)
            ):
                raise QuotaExhaustedError(message, body["quota"], body["resets_at"], retry_after)
            # The calling account's own limits.
            if code == "account_rate_limited":
                raise RateLimitedError(message, retry_after)
            if code == "account_credits_exhausted":
                raise CreditsExhaustedError(
                    message,
                    retry_after,
                    _str_or_none(err.get("next_grant_at")),
                    _str_or_none(err.get("credits_url")),
                )
        raise ChakraMCPError(response.status_code, code, message, retry_after)
    raise ChakraMCPError(
        response.status_code,
        "unknown",
        response.text or response.reason_phrase,
        _retry_after_secs(response, {}),
    )
