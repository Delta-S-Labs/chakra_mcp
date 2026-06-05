"""Swiggy Dineout MCP client.

URL + bearer token come from `.env` (`SWIGGY_MCP_URL`,
`SWIGGY_MCP_TOKEN`). If the token isn't set, we still construct a
None — the caller decides whether that's fatal (the demo flow only
needs Swiggy at the very end, after the cuisine + drink have been
agreed, so Swiggy unavailability degrades gracefully).
"""

from __future__ import annotations

import os

from agents.mcp import MCPServerStreamableHttp


def build_swiggy_mcp_server() -> MCPServerStreamableHttp | None:
    url = os.environ.get("SWIGGY_MCP_URL")
    token = os.environ.get("SWIGGY_MCP_TOKEN")
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
