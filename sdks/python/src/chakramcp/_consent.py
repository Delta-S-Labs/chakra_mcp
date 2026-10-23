"""Owner consent tokens: check that a person approved exactly this request,
or confirmed exactly this reply.

A consent token is an EdDSA JWT (``typ: "chakramcp-consent+jwt"``) signed
with the relay's key, published at ``{relay}/.well-known/jwks.json``. Its
``sha256`` is the SHA-256 of the RFC 8785 (JCS) canonical JSON of what was
delivered: the input (``kind: "request"``) or the released reply
``{status, output?, error?}`` (``kind: "reply"``).

Verifying the signature needs ``cryptography``:
``pip install "chakramcp[verify]"``.
"""

from __future__ import annotations

import base64
import hashlib
import json
import math
import time
from decimal import Decimal
from typing import Any, TypedDict

import httpx

CONSENT_TOKEN_TYPE = "chakramcp-consent+jwt"

_MISSING: Any = object()


class ConsentClaims(TypedDict, total=False):
    iss: str
    #: The consent request id.
    jti: str
    #: ``"request"`` or ``"reply"``.
    kind: str
    inv: str
    cap: str
    granter: str
    grantee: str
    approver: str
    #: The standing consent that covered the request, if one did.
    consent: str
    sha256: str
    iat: int
    exp: int


class ConsentTokenError(Exception):
    """A consent token didn't check out. ``code`` says why: ``malformed``,
    ``wrong_type``, ``unknown_key``, ``bad_signature``, ``expired`` or
    ``value_mismatch``."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


# ─── RFC 8785 (JCS) ──────────────────────────────────────


def _number(n: int | float) -> str:
    """ECMAScript Number serialization, as JCS requires."""
    if isinstance(n, bool):
        raise TypeError("booleans aren't numbers")
    if isinstance(n, int) and abs(n) < 2**53:
        return str(n)
    f = float(n)
    if not math.isfinite(f):
        raise ValueError("JCS can't encode a non-finite number")
    if f == 0:
        return "0"
    # The shortest digits that round-trip, as repr() gives them.
    sign, digits, exponent = Decimal(repr(f)).as_tuple()
    s = "".join(str(d) for d in digits).rstrip("0") or "0"
    k = len(s)
    # Position of the decimal point relative to the digits.
    n_pos = int(exponent) + len(digits)
    out: str
    if k <= n_pos <= 21:
        out = s + "0" * (n_pos - k)
    elif 0 < n_pos <= 21:
        out = s[:n_pos] + "." + s[n_pos:]
    elif -6 < n_pos <= 0:
        out = "0." + "0" * (-n_pos) + s
    else:
        e = n_pos - 1
        mantissa = s[0] + ("." + s[1:] if k > 1 else "")
        out = f"{mantissa}e{'+' if e > 0 else '-'}{abs(e)}"
    return ("-" if sign else "") + out


def canonical_json(value: Any) -> str:
    """RFC 8785 (JCS) canonical JSON: keys sorted by UTF-16 code units, no
    whitespace, ECMAScript number and string serialization."""
    if value is None:
        return "null"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, (int, float)):
        return _number(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, (list, tuple)):
        return "[" + ",".join(canonical_json(v) for v in value) + "]"
    if isinstance(value, dict):
        keys = sorted(value.keys(), key=lambda k: k.encode("utf-16-be"))
        return (
            "{"
            + ",".join(
                json.dumps(k, ensure_ascii=False) + ":" + canonical_json(value[k]) for k in keys
            )
            + "}"
        )
    raise TypeError(f"JCS can't encode {type(value).__name__}")


def sha256_jcs(value: Any) -> str:
    """Hex SHA-256 of the canonical JSON of ``value``."""
    return hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


# ─── Verifying ───────────────────────────────────────────


def _b64url(part: str) -> bytes:
    return base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))


def _decode(part: str) -> Any:
    try:
        return json.loads(_b64url(part))
    except Exception as err:
        raise ConsentTokenError("malformed", "the token isn't a JWT") from err


def _check(
    token: str, jwks: dict[str, Any], value: Any, now: float | None
) -> ConsentClaims:
    parts = token.split(".")
    if len(parts) != 3:
        raise ConsentTokenError("malformed", "the token isn't a JWT")
    header_b64, payload_b64, sig_b64 = parts
    header = _decode(header_b64)
    if header.get("typ") != CONSENT_TOKEN_TYPE or header.get("alg") != "EdDSA":
        raise ConsentTokenError("wrong_type", "not a ChakraMCP consent token")
    jwk = next((k for k in jwks.get("keys", []) if k.get("kid") == header.get("kid")), None)
    if not jwk or jwk.get("kty") != "OKP" or jwk.get("crv") != "Ed25519" or not jwk.get("x"):
        raise ConsentTokenError("unknown_key", "signed with a key the relay doesn't publish")
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    except ImportError as err:  # pragma: no cover - depends on the environment
        raise RuntimeError(
            'verifying consent tokens needs cryptography: pip install "chakramcp[verify]"'
        ) from err
    key = Ed25519PublicKey.from_public_bytes(_b64url(jwk["x"]))
    try:
        key.verify(_b64url(sig_b64), f"{header_b64}.{payload_b64}".encode())
    except InvalidSignature as err:
        raise ConsentTokenError("bad_signature", "the signature doesn't match") from err
    claims: ConsentClaims = _decode(payload_b64)
    if claims.get("exp", 0) <= (time.time() if now is None else now):
        raise ConsentTokenError("expired", "the token has expired")
    if value is not _MISSING and sha256_jcs(value) != claims.get("sha256"):
        raise ConsentTokenError("value_mismatch", "the token doesn't cover this value")
    return claims


def verify_consent_token(
    token: str,
    *,
    jwks_url: str | None = None,
    jwks: dict[str, Any] | None = None,
    value: Any = _MISSING,
    now: float | None = None,
) -> ConsentClaims:
    """Check a consent token: its type, its signature against the relay's
    keys (``jwks_url``, e.g. ``https://relay.chakramcp.com/.well-known/jwks.json``,
    or ``jwks``), its expiry and, given ``value`` (the input you were
    delivered, or the reply ``{status, output?, error?}``), that it covers
    exactly that. Returns the claims; raises :class:`ConsentTokenError`."""
    if jwks is None:
        if not jwks_url:
            raise TypeError("pass jwks_url or jwks")
        res = httpx.get(jwks_url, timeout=10)
        res.raise_for_status()
        jwks = res.json()
    return _check(token, jwks, value, now)


async def averify_consent_token(
    token: str,
    *,
    jwks_url: str | None = None,
    jwks: dict[str, Any] | None = None,
    value: Any = _MISSING,
    now: float | None = None,
) -> ConsentClaims:
    """:func:`verify_consent_token` for asyncio: fetches ``jwks_url`` without
    blocking."""
    if jwks is None:
        if not jwks_url:
            raise TypeError("pass jwks_url or jwks")
        async with httpx.AsyncClient(timeout=10) as client:
            res = await client.get(jwks_url)
            res.raise_for_status()
            jwks = res.json()
    return _check(token, jwks, value, now)
