"""ChakraMCP relay access — reuses the JWT minted by `chakramcp login`.

Rationale: the demo script tells each operator to log in with the CLI.
Rather than make the agent run a *second* OAuth dance for the relay's
MCP endpoint, we just lift the access token (or API key) the CLI
already stored and hand it to the OpenAI Agents SDK's MCP client as a
bearer header.

One login, two consumers. The CLI calls `relay.chakramcp.com/v1/*`
with it; we call `relay.chakramcp.com/mcp` with it.
"""

from __future__ import annotations

import os
import sys
import time
from dataclasses import dataclass
from pathlib import Path

if sys.version_info >= (3, 11):
    import tomllib
else:  # pragma: no cover
    import tomli as tomllib

from agents.mcp import MCPServerStreamableHttp


def _cli_config_path() -> Path:
    """Path the CLI's `directories::ProjectDirs` writes to per platform.

    Mirrors `backend/cli/src/config.rs::config_path`, which uses
    `ProjectDirs::from("com", "chakramcp", "chakramcp")`. The platform
    rules are: macOS — `~/Library/Application Support/<bundle>/`,
    Linux — `$XDG_CONFIG_HOME/<app>/` (or `~/.config/<app>/`),
    Windows — `%APPDATA%\\<org>\\<app>\\config\\`.
    """
    if sys.platform == "darwin":
        return (
            Path.home()
            / "Library/Application Support/com.chakramcp.chakramcp/config.toml"
        )
    if sys.platform == "win32":
        appdata = os.environ.get("APPDATA")
        if not appdata:
            raise RuntimeError("APPDATA not set; can't locate CLI config")
        return Path(appdata) / "chakramcp" / "chakramcp" / "config" / "config.toml"
    # Linux / BSD.
    cfg_home = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    return Path(cfg_home) / "chakramcp" / "config.toml"


@dataclass(frozen=True)
class CliSession:
    """The active network's credential as the CLI stored it."""

    relay_url: str
    bearer: str   # JWT access_token if fresh, else api_key
    method: str   # "access_token" | "api_key" — surfaces in logs

    def auth_header(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.bearer}"}


def load_cli_session() -> CliSession:
    """Read the CLI config and return the active network's credential.

    Prefers a non-expired access_token over the long-lived api_key,
    since revoking the token is one click in the app. Falls back to
    the api_key when no token is present.
    """
    path = _cli_config_path()
    if not path.exists():
        raise FileNotFoundError(
            f"no CLI config at {path}. Run `chakramcp login` first."
        )
    cfg = tomllib.loads(path.read_text())
    active_name = cfg.get("active")
    networks = cfg.get("networks", [])
    if not active_name or not networks:
        raise RuntimeError(
            "CLI config has no active network. Run `chakramcp login` first."
        )
    net = next((n for n in networks if n.get("name") == active_name), None)
    if net is None:
        raise RuntimeError(f"active network {active_name!r} not in config")
    auth = net.get("auth", {})
    token = auth.get("access_token")
    exp = auth.get("access_token_expires_at")
    if token and (exp is None or exp > int(time.time()) + 60):
        return CliSession(
            relay_url=net["relay_url"], bearer=token, method="access_token"
        )
    api_key = auth.get("api_key")
    if api_key:
        return CliSession(
            relay_url=net["relay_url"], bearer=api_key, method="api_key"
        )
    raise RuntimeError(
        "no usable access_token or api_key on the active network. "
        "Run `chakramcp login` again."
    )


def build_chakra_mcp_server(
    session: CliSession, mcp_url: str | None = None
) -> MCPServerStreamableHttp:
    """Construct the OpenAI Agents SDK MCP client for ChakraMCP /mcp.

    The relay's `/mcp` endpoint is a Streamable-HTTP MCP server (the
    current MCP transport; the older SSE form is deprecated). It's
    bearer-auth via the `Authorization` header, per the relay's
    `/.well-known/oauth-protected-resource`.
    """
    url = mcp_url or os.environ.get(
        "CHAKRAMCP_MCP_URL", "https://relay.chakramcp.com/mcp"
    )
    return MCPServerStreamableHttp(
        name="chakramcp",
        params={
            "url": url,
            "headers": session.auth_header(),
        },
        # cache_tools_list reduces noise in the logs tab: we want a tool
        # span per *call*, not per call + an extra list-tools poke.
        cache_tools_list=True,
    )
