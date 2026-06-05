"""Swiggy Dineout MCP client.

Swiggy is OAuth 2.1 (PKCE + DCR) — there's no static token to paste.
`swiggy_auth.ensure_swiggy_token()` runs the browser flow at startup
and returns a bearer; we just attach it here. A `None` token (Swiggy
not configured, or auth failed) means we build no server, and the
agent degrades gracefully — the demo only needs Swiggy at the very
end, after cuisine + drink are agreed.
"""

from __future__ import annotations

import os

from agents.mcp import MCPServerStreamableHttp


def swiggy_mcp_url() -> str | None:
    return os.environ.get("SWIGGY_MCP_URL")


def build_swiggy_mcp_server(token: str | None) -> MCPServerStreamableHttp | None:
    url = swiggy_mcp_url()
    if not url or not token:
        return None
    return MCPServerStreamableHttp(
        name="swiggy-dineout",
        params={
            "url": url,
            "headers": {"Authorization": f"Bearer {token}"},
        },
        cache_tools_list=True,
    )
