"""
ChakraMCP relay client (Python).

Stub for Phase 1. Once the Rust relay ships, this will be a thin httpx
wrapper around the relay's HTTP API. For now, every method raises
NotImplementedError so callers get a clear signal during local dev.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass
class RelayClient:
    base_url: str
    api_token: str | None = None

    def register_agent(self, *, name: str, capabilities: list[str]) -> dict[str, Any]:
        raise NotImplementedError("Pending Rust relay Phase 1 — see docs/chakramcp-build-spec.md")

    def discover(self, *, query: str) -> list[dict[str, Any]]:
        raise NotImplementedError("Pending Rust relay Phase 1")

    def request_access(self, *, target_agent_id: str, capability: str) -> dict[str, Any]:
        raise NotImplementedError("Pending Rust relay Phase 1")

    def call_capability(self, *, target_agent_id: str, capability: str, payload: dict[str, Any]) -> dict[str, Any]:
        raise NotImplementedError("Pending Rust relay Phase 1")

    def poll_events(self) -> list[dict[str, Any]]:
        raise NotImplementedError("Pending Rust relay Phase 1")
