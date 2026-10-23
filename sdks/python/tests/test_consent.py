"""Consent tokens and RFC 8785 canonical JSON."""

from __future__ import annotations

import base64
import json
import time
from pathlib import Path
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

from chakramcp import (
    ConsentTokenError,
    averify_consent_token,
    canonical_json,
    sha256_jcs,
    verify_consent_token,
)

# The relay's RFC 8785 vectors (backend/relay/tests/data/jcs/vectors.json):
# every implementation must produce the same bytes.
VECTORS = json.loads((Path(__file__).parent / "fixtures" / "jcs-vectors.json").read_text())


@pytest.mark.parametrize(("source", "canonical"), VECTORS)
def test_canonical_json_matches_the_relay_vectors(source: str, canonical: str) -> None:
    assert canonical_json(json.loads(source)) == canonical


def test_canonical_json_ignores_key_order_and_formats_numbers_as_js_does() -> None:
    a = json.loads('{"b": 1, "a": [1, 2, {"d": "x", "c": null}]}')
    b = json.loads('{"a":[1,2,{"c":null,"d":"x"}],"b":1}')
    assert sha256_jcs(a) == sha256_jcs(b)
    assert canonical_json([1.0, 0.1, 1e21, 1e-7, -0.0, 2**60]) == (
        "[1,0.1,1e+21,1e-7,0,1152921504606847000]"
    )


def _b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def _signed(claims: dict[str, Any], typ: str = "chakramcp-consent+jwt") -> tuple[str, dict[str, Any]]:
    key = Ed25519PrivateKey.generate()
    x = key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    header = _b64url(json.dumps({"alg": "EdDSA", "kid": "k1", "typ": typ}).encode())
    payload = _b64url(json.dumps(claims).encode())
    sig = _b64url(key.sign(f"{header}.{payload}".encode()))
    jwks = {"keys": [{"kty": "OKP", "crv": "Ed25519", "kid": "k1", "x": _b64url(x)}]}
    return f"{header}.{payload}.{sig}", jwks


INPUT = {"date": "2026-10-28", "minutes": 45}
NOW = int(time.time())


def test_accepts_a_token_for_exactly_this_value() -> None:
    token, jwks = _signed(
        {"kind": "request", "jti": "cr-1", "sha256": sha256_jcs(INPUT), "iat": NOW, "exp": NOW + 3600}
    )
    claims = verify_consent_token(token, jwks=jwks, value={"minutes": 45, "date": "2026-10-28"})
    assert claims["jti"] == "cr-1"
    with pytest.raises(ConsentTokenError) as err:
        verify_consent_token(token, jwks=jwks, value={**INPUT, "minutes": 60})
    assert err.value.code == "value_mismatch"


@pytest.mark.parametrize(
    ("make", "code"),
    [
        (lambda: _signed({"exp": NOW + 60}, typ="JWT"), "wrong_type"),
        (lambda: _signed({"exp": NOW - 1}), "expired"),
        (lambda: (_signed({"exp": NOW + 60})[0], _signed({"exp": NOW + 60})[1]), "bad_signature"),
        (lambda: (_signed({"exp": NOW + 60})[0], {"keys": []}), "unknown_key"),
        (lambda: ("not-a-token", {"keys": []}), "malformed"),
    ],
)
def test_refuses_tokens_that_dont_check_out(make: Any, code: str) -> None:
    token, jwks = make()
    with pytest.raises(ConsentTokenError) as err:
        verify_consent_token(token, jwks=jwks)
    assert err.value.code == code


async def test_async_fetches_the_relay_keys(respx_mock: Any) -> None:
    token, jwks = _signed({"kind": "reply", "sha256": sha256_jcs({"status": "succeeded"}), "exp": NOW + 60})
    url = "http://relay.test/.well-known/jwks.json"
    respx_mock.get(url).respond(200, json=jwks)
    claims = await averify_consent_token(token, jwks_url=url, value={"status": "succeeded"})
    assert claims["kind"] == "reply"
