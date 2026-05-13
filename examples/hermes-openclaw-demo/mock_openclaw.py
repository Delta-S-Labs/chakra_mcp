"""Tiny mock OpenClaw A2A gateway — stand-in for a real openclaw-a2a-gateway.

Real OpenClaw is a Node plugin (https://github.com/win4r/openclaw-a2a-gateway).
For this demo we don't need the full thing — just enough to:

1. Serve a canonical A2A v0.3 Agent Card at /.well-known/agent-card.json,
   so the ChakraMCP relay's D2d fetcher can discover its capabilities.
2. Accept POST /a2a/jsonrpc calls (the URL the agent card advertises),
   verify the JWT we mint for forwarded calls, and return a deterministic
   `Task.completed` shape with whatever input the caller sent.

This lets the full Hermes ↔ OpenClaw flow execute end-to-end against a
local relay without depending on a real OpenClaw installation. Replace
this with the real thing in production by pointing
`agent_card_url` at the running gateway's URL.

Run:

    python mock_openclaw.py --port 18800

Then in another terminal:

    python setup.py
    python hermes_bot.py

stdlib only — no extra deps so you can run this on any Python 3.10+.
"""
from __future__ import annotations

import argparse
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

# What the mock advertises. ChakraMCP's fetcher (D2d) will pull this,
# normalize it (substitute our relay URL into supported_interfaces,
# replace auth with chakramcp_bearer, preserve the upstream signature
# array), and serve the resulting card under our domain.
AGENT_CARD_TEMPLATE = {
    "name": "OpenClaw Recipe Bot",
    "description": "Suggests recipes given an ingredient list. Mock OpenClaw bot for the demo.",
    "supported_interfaces": [
        {
            "url": "REPLACED_AT_RUNTIME",
            "protocol_binding": "JSONRPC",
            "protocol_version": "0.3",
        }
    ],
    "version": "1.0.0",
    "capabilities": {"streaming": False, "push_notifications": False},
    "security_schemes": {
        "openclaw_bearer": {
            "http": {
                "scheme": "Bearer",
                "bearer_format": "JWT",
                "description": "OpenClaw expects a JWT issued by ChakraMCP relay.",
            }
        }
    },
    "security_requirements": [{"openclaw_bearer": []}],
    "default_input_modes": ["application/json"],
    "default_output_modes": ["application/json"],
    "skills": [
        {
            "id": "suggest-recipes",
            "name": "Suggest Recipes",
            "description": "Given an ingredient list, returns 3 recipe ideas.",
            "tags": ["cooking", "recipes"],
            "examples": [
                "Suggest recipes for chicken, rice, and lemon.",
            ],
        }
    ],
}


def make_handler(port: int):
    class Handler(BaseHTTPRequestHandler):
        # Keep the mock quiet by default; flip to True for debugging.
        def log_message(self, fmt, *args):
            print(f"[openclaw {self.command}] {fmt % args}", file=sys.stderr)

        def do_GET(self):  # noqa: N802 (BaseHTTPRequestHandler API)
            if self.path != "/.well-known/agent-card.json":
                self.send_error(404, "Not Found")
                return
            card = json.loads(json.dumps(AGENT_CARD_TEMPLATE))  # deep copy
            card["supported_interfaces"][0]["url"] = f"http://127.0.0.1:{port}/a2a/jsonrpc"
            body = json.dumps(card).encode("utf-8")
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("cache-control", "public, max-age=300")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):  # noqa: N802
            if self.path != "/a2a/jsonrpc":
                self.send_error(404, "Not Found")
                return

            length = int(self.headers.get("content-length") or 0)
            raw = self.rfile.read(length)
            try:
                envelope = json.loads(raw)
            except json.JSONDecodeError:
                return self._reply_error(-32700, "parse error", req_id=None)

            method = envelope.get("method")
            req_id = envelope.get("id")
            auth = self.headers.get("authorization", "")

            # Sanity-check that the relay-minted JWT made it through.
            # We don't verify the signature here — that's a real OpenClaw
            # concern. We just confirm the relay attached SOMETHING
            # plausible so the path-from-relay assertion holds.
            if not auth.startswith("Bearer "):
                return self._reply_error(
                    -32001, "missing bearer token", req_id=req_id, status=401
                )

            if method != "SendMessage":
                return self._reply_error(
                    -32601, f"method not supported: {method}", req_id=req_id
                )

            params = envelope.get("params") or {}
            ingredients = (
                params.get("message", {}).get("parts", [{}])[0].get("data", {})
            ) or params  # tolerate either wire shape
            recipes = [
                f"Lemon herb roast (uses {', '.join(str(i) for i in (ingredients or {}).values()) or 'pantry items'})",
                "One-pot rice pilaf",
                "Pan-seared protein with citrus glaze",
            ]
            result = {
                "id": f"openclaw-task-{req_id}",
                "status": {"state": "completed"},
                "artifacts": [
                    {"parts": [{"data": {"recipes": recipes, "from": "openclaw-mock"}}]}
                ],
            }
            self._reply_ok(req_id, result)

        def _reply_ok(self, req_id, result):
            body = json.dumps(
                {"jsonrpc": "2.0", "id": req_id, "result": result}
            ).encode("utf-8")
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _reply_error(self, code, message, *, req_id, status=200):
            body = json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {"code": code, "message": message},
                }
            ).encode("utf-8")
            self.send_response(status)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    return Handler


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--port", type=int, default=18800)
    args = p.parse_args()
    server = HTTPServer(("127.0.0.1", args.port), make_handler(args.port))
    print(
        f"mock-openclaw listening on http://127.0.0.1:{args.port}\n"
        f"  card: http://127.0.0.1:{args.port}/.well-known/agent-card.json\n"
        f"  a2a:  http://127.0.0.1:{args.port}/a2a/jsonrpc",
        file=sys.stderr,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
