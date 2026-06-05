"""Swiggy Dineout OAuth 2.1 (PKCE + Dynamic Client Registration).

Swiggy MCP uses standard MCP-spec auth — OAuth 2.1, public client,
PKCE S256, no static token. There's nothing to paste from a
dashboard, so the agent obtains a token itself at startup the first
time, then caches it (the access token lasts ~5 days; refresh isn't
wired in Swiggy v1.0, so we just re-run the browser flow when it
lapses).

Flow (RFC 8252 native-app loopback):
  1. Discover endpoints from
     /.well-known/oauth-authorization-server  (verified live).
  2. Dynamic Client Registration → client_id  (public client).
  3. PKCE: code_verifier + S256 challenge.
  4. Open the browser to /auth/authorize with a loopback redirect.
  5. Catch the redirect on 127.0.0.1:<port> → authorization code.
  6. Exchange code at /auth/token → access_token.
  7. Cache {client_id, access_token, expires_at} to disk.

The whole thing runs *before* the Textual UI takes over the screen,
so the browser handoff is clean. On later runs within the token's
life, the cache short-circuits all of this.

Endpoints are *discovered*, not hardcoded — but the live values at
build time were:
  authorize: https://mcp.swiggy.com/auth/authorize
  token:     https://mcp.swiggy.com/auth/token
  register:  https://mcp.swiggy.com/auth/register
  scopes:    mcp:tools mcp:resources mcp:prompts
"""

from __future__ import annotations

import base64
import hashlib
import http.server
import json
import os
import secrets
import threading
import time
import urllib.parse
import webbrowser
from dataclasses import dataclass
from pathlib import Path

import httpx

DISCOVERY = "/.well-known/oauth-authorization-server"
SCOPES = "mcp:tools mcp:resources mcp:prompts"
CLIENT_NAME = "chakramcp-voice-agent-demo"
# Buffer so we re-auth slightly before the server would reject us.
EXPIRY_BUFFER_S = 600


def _cache_path(persona_name: str) -> Path:
    """Per-persona token cache (so two personas on one machine don't
    clobber each other's Swiggy session)."""
    base = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    d = Path(base) / "voice-agent-demo"
    d.mkdir(parents=True, exist_ok=True)
    return d / f"swiggy_token_{persona_name}.json"


@dataclass
class SwiggyToken:
    access_token: str
    expires_at: float

    def valid(self) -> bool:
        return bool(self.access_token) and self.expires_at > time.time() + EXPIRY_BUFFER_S


# ─── PKCE ────────────────────────────────────────────────────────


def _pkce() -> tuple[str, str]:
    verifier = base64.urlsafe_b64encode(secrets.token_bytes(48)).rstrip(b"=").decode()
    challenge = (
        base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest())
        .rstrip(b"=")
        .decode()
    )
    return verifier, challenge


# ─── Loopback redirect catcher ───────────────────────────────────


class _CallbackHandler(http.server.BaseHTTPRequestHandler):
    code: str | None = None
    state: str | None = None
    error: str | None = None

    def do_GET(self) -> None:  # noqa: N802
        q = urllib.parse.urlparse(self.path).query
        params = urllib.parse.parse_qs(q)
        _CallbackHandler.code = (params.get("code") or [None])[0]
        _CallbackHandler.state = (params.get("state") or [None])[0]
        _CallbackHandler.error = (params.get("error") or [None])[0]
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.end_headers()
        msg = (
            "Swiggy login complete — you can close this tab and return to the terminal."
            if _CallbackHandler.error is None
            else f"Swiggy login failed: {_CallbackHandler.error}"
        )
        self.wfile.write(
            f"<html><body style='font-family:sans-serif;padding:3rem'>"
            f"<h2>{msg}</h2></body></html>".encode()
        )

    def log_message(self, *args) -> None:  # silence the default stderr spam
        return


def _capture_code(port: int, expected_state: str, timeout_s: float = 300) -> str:
    """Run a one-shot loopback server until the redirect arrives."""
    _CallbackHandler.code = _CallbackHandler.state = _CallbackHandler.error = None
    server = http.server.HTTPServer(("127.0.0.1", port), _CallbackHandler)
    server.timeout = 1
    deadline = time.time() + timeout_s
    try:
        while time.time() < deadline:
            server.handle_request()
            if _CallbackHandler.code or _CallbackHandler.error:
                break
    finally:
        server.server_close()
    if _CallbackHandler.error:
        raise RuntimeError(f"authorization failed: {_CallbackHandler.error}")
    if not _CallbackHandler.code:
        raise TimeoutError("timed out waiting for the Swiggy redirect")
    if _CallbackHandler.state != expected_state:
        raise RuntimeError("state mismatch on the OAuth redirect (possible CSRF)")
    return _CallbackHandler.code


def _free_port() -> int:
    import socket

    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


# ─── The flow ────────────────────────────────────────────────────


def _server_origin(mcp_url: str) -> str:
    p = urllib.parse.urlparse(mcp_url)
    return f"{p.scheme}://{p.netloc}"


def _discover(origin: str) -> dict:
    r = httpx.get(origin + DISCOVERY, timeout=15)
    r.raise_for_status()
    return r.json()


def _register_client(reg_endpoint: str, redirect_uri: str) -> str:
    body = {
        "client_name": CLIENT_NAME,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",  # public client
        "scope": SCOPES,
    }
    r = httpx.post(reg_endpoint, json=body, timeout=20)
    r.raise_for_status()
    data = r.json()
    cid = data.get("client_id")
    if not cid:
        raise RuntimeError(f"DCR response had no client_id: {data}")
    return cid


def _run_browser_flow(mcp_url: str, on_status=lambda s: None) -> SwiggyToken:
    origin = _server_origin(mcp_url)
    meta = _discover(origin)
    authorize = meta["authorization_endpoint"]
    token_ep = meta["token_endpoint"]
    register = meta["registration_endpoint"]

    port = _free_port()
    redirect_uri = f"http://127.0.0.1:{port}/callback"

    on_status("registering client…")
    client_id = _register_client(register, redirect_uri)

    verifier, challenge = _pkce()
    state = secrets.token_urlsafe(16)
    auth_url = authorize + "?" + urllib.parse.urlencode(
        {
            "response_type": "code",
            "client_id": client_id,
            "redirect_uri": redirect_uri,
            "scope": SCOPES,
            "state": state,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
            # RFC 8707 resource indicator — bind the token to this MCP resource.
            "resource": mcp_url,
        }
    )

    on_status("opening browser for Swiggy login…")
    # Print the URL too, in case the browser can't auto-open (headless box).
    print(f"\n  → If a browser didn't open, visit:\n    {auth_url}\n")
    try:
        webbrowser.open(auth_url)
    except Exception:
        pass

    code = _capture_code(port, state)

    on_status("exchanging code for token…")
    r = httpx.post(
        token_ep,
        data={
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": redirect_uri,
            "client_id": client_id,
            "code_verifier": verifier,
            "resource": mcp_url,
        },
        timeout=20,
    )
    r.raise_for_status()
    tok = r.json()
    access = tok.get("access_token")
    if not access:
        raise RuntimeError(f"token response had no access_token: {tok}")
    expires_in = int(tok.get("expires_in", 5 * 24 * 3600))  # default to docs' 5d
    return SwiggyToken(access_token=access, expires_at=time.time() + expires_in)


# ─── Public entry ────────────────────────────────────────────────


def ensure_swiggy_token(
    persona_name: str, mcp_url: str | None, on_status=lambda s: None
) -> str | None:
    """Return a valid Swiggy bearer token, running the browser OAuth
    flow if there's no fresh cached one. Returns None when Swiggy is
    not configured (no MCP URL) — the caller treats that as "Swiggy
    disabled" and the agent degrades gracefully.
    """
    if not mcp_url:
        return None

    cache = _cache_path(persona_name)
    if cache.exists():
        try:
            data = json.loads(cache.read_text())
            cached = SwiggyToken(data["access_token"], float(data["expires_at"]))
            if cached.valid():
                on_status("using cached Swiggy token")
                return cached.access_token
        except Exception:
            pass  # corrupt cache → just re-auth

    token = _run_browser_flow(mcp_url, on_status)
    cache.write_text(
        json.dumps({"access_token": token.access_token, "expires_at": token.expires_at})
    )
    # Lock down — it's a bearer token.
    try:
        cache.chmod(0o600)
    except OSError:
        pass
    on_status("Swiggy authenticated")
    return token.access_token
