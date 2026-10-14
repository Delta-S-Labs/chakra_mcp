#!/usr/bin/env python3
"""The end-to-end test's MCP client: connects to a ChakraMCP relay the way
an MCP client does, with nothing but the relay's URL.

1. An unauthenticated call gets a 401 naming the protected-resource
   metadata (WWW-Authenticate).
2. The metadata names the authorization server; its metadata names the
   endpoints.
3. Register a client (RFC 7591), then authorize with PKCE: sign in and
   approve on the server's pages, with a local listener catching the code.
4. Exchange the code, then `initialize` and `tools/list` with the token.

Usage: mcp_client.py <relay URL>. Credentials come from E2E_ADMIN_EMAIL and
E2E_ADMIN_PASSWORD.
"""

import base64
import hashlib
import http.server
import json
import os
import re
import secrets
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pages  # noqa: E402


def http_json(method, url, body=None, headers=None, form=False):
    headers = dict(headers or {})
    data = None
    if body is not None:
        if form:
            data = urllib.parse.urlencode(body).encode()
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        else:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, response.headers, json.loads(response.read() or b"null")
    except urllib.error.HTTPError as err:
        raw = err.read()
        try:
            payload = json.loads(raw or b"null")
        except ValueError:
            payload = raw.decode("utf-8", "replace")
        return err.code, err.headers, payload


def catch_one_callback():
    """A loopback listener for one redirect; returns (redirect_uri, result)."""
    result = {}

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            result["query"] = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(b"done")

        def log_message(self, *args):
            pass

    server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.handle_request, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}/callback", result


def main():
    relay = sys.argv[1].rstrip("/")
    email = os.environ["E2E_ADMIN_EMAIL"]
    password = os.environ["E2E_ADMIN_PASSWORD"]
    initialize = {
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "chakramcp-e2e", "version": "0"}},
    }

    status, headers, _ = http_json("POST", f"{relay}/mcp", initialize)
    assert status == 401, f"an unauthenticated call should get 401, got {status}"
    challenge = headers.get("WWW-Authenticate", "")
    match = re.search(r'resource_metadata="([^"]+)"', challenge)
    assert match, f"no resource_metadata in WWW-Authenticate: {challenge!r}"
    print(f"401 names {match.group(1)}")

    _, _, resource = http_json("GET", match.group(1))
    _, _, path_form = http_json("GET", f"{relay}/.well-known/oauth-protected-resource/mcp")
    assert path_form == resource, "the path-form metadata should match"
    issuer = resource["authorization_servers"][0]
    _, _, server_meta = http_json("GET", f"{issuer}/.well-known/oauth-authorization-server")
    print(f"authorization server {issuer}")

    redirect_uri, callback = catch_one_callback()
    status, _, client = http_json("POST", server_meta["registration_endpoint"], {
        "redirect_uris": [redirect_uri], "client_name": "E2E MCP client",
    })
    assert status == 200, f"registration failed: {status} {client}"

    verifier = secrets.token_urlsafe(48)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip("=")
    state = secrets.token_urlsafe(8)
    authorize_url = server_meta["authorization_endpoint"] + "?" + urllib.parse.urlencode({
        "response_type": "code", "client_id": client["client_id"], "redirect_uri": redirect_uri,
        "code_challenge": challenge, "code_challenge_method": "S256", "state": state,
        "scope": "relay.full",
    })
    pages.authorize(pages.Browser(), authorize_url, email, password, agent_scope="own")
    query = callback.get("query") or {}
    assert query.get("state") == [state], f"state didn't round-trip: {query}"
    code = query["code"][0]

    status, _, token = http_json("POST", server_meta["token_endpoint"], {
        "grant_type": "authorization_code", "code": code, "client_id": client["client_id"],
        "redirect_uri": redirect_uri, "code_verifier": verifier,
    }, form=True)
    assert status == 200, f"token exchange failed: {status} {token}"
    auth = {"Authorization": f"Bearer {token['access_token']}"}

    status, _, init = http_json("POST", f"{relay}/mcp", initialize, auth)
    assert status == 200 and "result" in init, f"initialize failed: {status} {init}"
    print(f"initialize: {init['result']['serverInfo']}")
    status, _, tools = http_json("POST", f"{relay}/mcp", {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}, auth)
    names = [t["name"] for t in tools["result"]["tools"]]
    assert names, f"tools/list returned no tools: {tools}"
    print(f"tools/list: {len(names)} tools ({', '.join(names[:5])}, ...)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
