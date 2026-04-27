"""Synchronous client. Use this in scripts, notebooks, and code paths
that aren't already in an asyncio event loop. For long-running agent
workers see :mod:`._async`.
"""

from __future__ import annotations

import time
from collections.abc import Callable
from typing import Any

import httpx

from ._errors import raise_for_response
from ._types import (
    TERMINAL_STATUSES,
    Agent,
    Capability,
    CreateAgentRequest,
    CreateCapabilityRequest,
    CreateGrantRequest,
    Friendship,
    FriendshipStatus,
    Grant,
    GrantStatus,
    HandlerResult,
    Invocation,
    InvocationStatus,
    InvokeResponse,
    MeResponse,
    ProposeFriendshipRequest,
    UpdateAgentRequest,
)

DEFAULT_APP_URL = "https://chakramcp.com"
DEFAULT_RELAY_URL = "https://relay.chakramcp.com"
USER_AGENT = "chakramcp-python-sdk"


class ChakraMCP:
    """Synchronous client.

    .. code-block:: python

        from chakramcp import ChakraMCP
        chakra = ChakraMCP(api_key=os.environ["CHAKRAMCP_API_KEY"])
        me = chakra.me()
    """

    def __init__(
        self,
        *,
        api_key: str,
        app_url: str = DEFAULT_APP_URL,
        relay_url: str = DEFAULT_RELAY_URL,
        timeout: float = 60.0,
        http_client: httpx.Client | None = None,
    ) -> None:
        if not api_key or not api_key.startswith("ck_"):
            raise ValueError("api_key must be a `ck_…` API key")
        self.app_url = app_url.rstrip("/")
        self.relay_url = relay_url.rstrip("/")
        self._owns_client = http_client is None
        self._http = http_client or httpx.Client(
            timeout=timeout,
            headers={
                "authorization": f"Bearer {api_key}",
                "user-agent": USER_AGENT,
            },
        )
        self.agents = AgentsClient(self)
        self.friendships = FriendshipsClient(self)
        self.grants = GrantsClient(self)
        self.invocations = InvocationsClient(self)
        self.inbox = InboxClient(self)

    def close(self) -> None:
        if self._owns_client:
            self._http.close()

    def __enter__(self) -> ChakraMCP:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    # ─── Internal request helpers ────────────────────

    def _request(self, base_url: str, method: str, path: str, json: Any | None = None) -> Any:
        resp = self._http.request(method, f"{base_url}{path}", json=json)
        raise_for_response(resp)
        if resp.status_code == 204 or not resp.content:
            return None
        return resp.json()

    def _app(self, method: str, path: str, json: Any | None = None) -> Any:
        return self._request(self.app_url, method, path, json)

    def _relay(self, method: str, path: str, json: Any | None = None) -> Any:
        return self._request(self.relay_url, method, path, json)

    # ─── Top-level ───────────────────────────────────

    def me(self) -> MeResponse:
        return self._app("GET", "/v1/me")

    def network(self) -> list[Agent]:
        return self._relay("GET", "/v1/network/agents")

    def invoke(self, body: dict[str, Any]) -> InvokeResponse:
        """Enqueue an invocation. Returns immediately; use
        :py:meth:`invoke_and_wait` to also poll until terminal.
        """
        return self._relay("POST", "/v1/invoke", body)

    def invoke_and_wait(
        self,
        body: dict[str, Any],
        *,
        interval_s: float = 1.5,
        timeout_s: float = 180.0,
    ) -> Invocation:
        """Enqueue an invocation and poll until status is terminal.

        Raises :py:class:`TimeoutError` after ``timeout_s`` seconds —
        the invocation may still be in flight; check it later via
        ``chakra.invocations.get(id)`` or the audit log.
        """
        deadline = time.monotonic() + timeout_s
        enqueued = self.invoke(body)
        if enqueued["status"] in TERMINAL_STATUSES:
            return self.invocations.get(enqueued["invocation_id"])
        while time.monotonic() < deadline:
            time.sleep(interval_s)
            fresh = self.invocations.get(enqueued["invocation_id"])
            if fresh["status"] in TERMINAL_STATUSES:
                return fresh
        raise TimeoutError(
            f"invoke_and_wait timed out after {timeout_s}s — invocation "
            f"{enqueued['invocation_id']} is still in flight"
        )


# ─── Sub-clients ─────────────────────────────────────────


class AgentsClient:
    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra
        self.capabilities = _CapabilitiesClient(chakra)

    def list(self) -> list[Agent]:
        return self._c._relay("GET", "/v1/agents")

    def get(self, agent_id: str) -> Agent:
        return self._c._relay("GET", f"/v1/agents/{agent_id}")

    def create(self, body: CreateAgentRequest | dict[str, Any]) -> Agent:
        return self._c._relay("POST", "/v1/agents", dict(body))

    def update(self, agent_id: str, body: UpdateAgentRequest | dict[str, Any]) -> Agent:
        return self._c._relay("PATCH", f"/v1/agents/{agent_id}", dict(body))

    def delete(self, agent_id: str) -> None:
        self._c._relay("DELETE", f"/v1/agents/{agent_id}")


class _CapabilitiesClient:
    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra

    def list(self, agent_id: str) -> list[Capability]:
        return self._c._relay("GET", f"/v1/agents/{agent_id}/capabilities")

    def create(
        self, agent_id: str, body: CreateCapabilityRequest | dict[str, Any]
    ) -> Capability:
        return self._c._relay("POST", f"/v1/agents/{agent_id}/capabilities", dict(body))

    def delete(self, agent_id: str, capability_id: str) -> None:
        self._c._relay("DELETE", f"/v1/agents/{agent_id}/capabilities/{capability_id}")


class FriendshipsClient:
    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra

    def list(
        self,
        *,
        direction: str | None = None,
        status: FriendshipStatus | None = None,
    ) -> list[Friendship]:
        params = {k: v for k, v in {"direction": direction, "status": status}.items() if v}
        suffix = "?" + "&".join(f"{k}={v}" for k, v in params.items()) if params else ""
        return self._c._relay("GET", f"/v1/friendships{suffix}")

    def get(self, friendship_id: str) -> Friendship:
        return self._c._relay("GET", f"/v1/friendships/{friendship_id}")

    def propose(self, body: ProposeFriendshipRequest | dict[str, Any]) -> Friendship:
        return self._c._relay("POST", "/v1/friendships", dict(body))

    def accept(self, friendship_id: str, *, message: str | None = None) -> Friendship:
        return self._c._relay(
            "POST", f"/v1/friendships/{friendship_id}/accept", {"response_message": message}
        )

    def reject(self, friendship_id: str, *, message: str | None = None) -> Friendship:
        return self._c._relay(
            "POST", f"/v1/friendships/{friendship_id}/reject", {"response_message": message}
        )

    def counter(self, friendship_id: str, *, message: str) -> Friendship:
        return self._c._relay(
            "POST",
            f"/v1/friendships/{friendship_id}/counter",
            {"proposer_message": message},
        )

    def cancel(self, friendship_id: str) -> Friendship:
        return self._c._relay("POST", f"/v1/friendships/{friendship_id}/cancel", {})


class GrantsClient:
    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra

    def list(
        self,
        *,
        direction: str | None = None,
        status: GrantStatus | None = None,
    ) -> list[Grant]:
        params = {k: v for k, v in {"direction": direction, "status": status}.items() if v}
        suffix = "?" + "&".join(f"{k}={v}" for k, v in params.items()) if params else ""
        return self._c._relay("GET", f"/v1/grants{suffix}")

    def get(self, grant_id: str) -> Grant:
        return self._c._relay("GET", f"/v1/grants/{grant_id}")

    def create(self, body: CreateGrantRequest | dict[str, Any]) -> Grant:
        return self._c._relay("POST", "/v1/grants", dict(body))

    def revoke(self, grant_id: str, *, reason: str | None = None) -> Grant:
        return self._c._relay("POST", f"/v1/grants/{grant_id}/revoke", {"reason": reason})


class InvocationsClient:
    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra

    def list(
        self,
        *,
        direction: str | None = None,
        agent_id: str | None = None,
        status: InvocationStatus | None = None,
    ) -> list[Invocation]:
        params: dict[str, str] = {}
        if direction:
            params["direction"] = direction
        if agent_id:
            params["agent_id"] = agent_id
        if status:
            params["status"] = status
        suffix = "?" + "&".join(f"{k}={v}" for k, v in params.items()) if params else ""
        return self._c._relay("GET", f"/v1/invocations{suffix}")

    def get(self, invocation_id: str) -> Invocation:
        return self._c._relay("GET", f"/v1/invocations/{invocation_id}")


class InboxClient:
    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra

    def pull(self, agent_id: str, *, limit: int | None = None) -> list[Invocation]:
        params: dict[str, str] = {"agent_id": agent_id}
        if limit is not None:
            params["limit"] = str(limit)
        suffix = "&".join(f"{k}={v}" for k, v in params.items())
        return self._c._relay("GET", f"/v1/inbox?{suffix}")

    def respond(self, invocation_id: str, result: HandlerResult | dict[str, Any]) -> Invocation:
        return self._c._relay(
            "POST", f"/v1/invocations/{invocation_id}/result", dict(result)
        )

    def serve(
        self,
        agent_id: str,
        handler: Callable[[Invocation], HandlerResult | dict[str, Any]],
        *,
        poll_interval_s: float = 2.0,
        batch_size: int = 25,
        on_error: Callable[[BaseException, Invocation | None], None] | None = None,
        stop: Callable[[], bool] | None = None,
    ) -> None:
        """Long-running pull → handler → respond loop.

        ``handler`` returns either ``{"status": "succeeded", "output": …}``
        or ``{"status": "failed", "error": "…"}``. Exceptions raised by
        the handler are caught and reported as failed; the loop keeps
        going. Pass ``stop=lambda: shutdown_event.is_set()`` to bail
        cleanly between iterations.
        """
        while not (stop and stop()):
            try:
                batch = self.pull(agent_id, limit=batch_size)
            except BaseException as err:
                if on_error:
                    on_error(err, None)
                time.sleep(poll_interval_s)
                continue
            if not batch:
                time.sleep(poll_interval_s)
                continue
            for inv in batch:
                if stop and stop():
                    return
                try:
                    result = handler(inv)
                    self.respond(inv["id"], result)
                except BaseException as err:
                    if on_error:
                        on_error(err, inv)
                    try:
                        self.respond(inv["id"], {"status": "failed", "error": str(err)})
                    except BaseException as inner:
                        if on_error:
                            on_error(inner, inv)
