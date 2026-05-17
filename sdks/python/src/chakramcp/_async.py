"""Async client. Use this in async agent code; the API surface mirrors
the sync :class:`ChakraMCP` exactly with ``await`` on every call.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
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


class AsyncChakraMCP:
    """Async client.

    .. code-block:: python

        from chakramcp import AsyncChakraMCP
        async with AsyncChakraMCP(api_key=...) as chakra:
            me = await chakra.me()
            await chakra.inbox.serve(agent_id, handler)
    """

    def __init__(
        self,
        *,
        api_key: str,
        app_url: str = DEFAULT_APP_URL,
        relay_url: str = DEFAULT_RELAY_URL,
        timeout: float = 60.0,
        http_client: httpx.AsyncClient | None = None,
    ) -> None:
        if not api_key or not api_key.startswith("ck_"):
            raise ValueError("api_key must be a `ck_…` API key")
        self.app_url = app_url.rstrip("/")
        self.relay_url = relay_url.rstrip("/")
        self._owns_client = http_client is None
        self._http = http_client or httpx.AsyncClient(
            timeout=timeout,
            headers={
                "authorization": f"Bearer {api_key}",
                "user-agent": USER_AGENT,
            },
        )
        self.agents = AsyncAgentsClient(self)
        self.friendships = AsyncFriendshipsClient(self)
        self.grants = AsyncGrantsClient(self)
        self.invocations = AsyncInvocationsClient(self)
        self.inbox = AsyncInboxClient(self)

    async def aclose(self) -> None:
        if self._owns_client:
            await self._http.aclose()

    async def __aenter__(self) -> AsyncChakraMCP:
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.aclose()

    async def _request(
        self, base_url: str, method: str, path: str, json: Any | None = None
    ) -> Any:
        resp = await self._http.request(method, f"{base_url}{path}", json=json)
        raise_for_response(resp)
        if resp.status_code == 204 or not resp.content:
            return None
        return resp.json()

    async def _app(self, method: str, path: str, json: Any | None = None) -> Any:
        return await self._request(self.app_url, method, path, json)

    async def _relay(self, method: str, path: str, json: Any | None = None) -> Any:
        return await self._request(self.relay_url, method, path, json)

    async def me(self) -> MeResponse:
        return await self._app("GET", "/v1/me")

    async def network(self) -> list[Agent]:
        return await self._relay("GET", "/v1/network/agents")

    async def invoke(self, body: dict[str, Any]) -> InvokeResponse:
        return await self._relay("POST", "/v1/invoke", body)

    async def invoke_and_wait(
        self,
        body: dict[str, Any],
        *,
        interval_s: float = 1.5,
        timeout_s: float = 180.0,
    ) -> Invocation:
        deadline = asyncio.get_event_loop().time() + timeout_s
        enqueued = await self.invoke(body)
        if enqueued["status"] in TERMINAL_STATUSES:
            return await self.invocations.get(enqueued["invocation_id"])
        while asyncio.get_event_loop().time() < deadline:
            await asyncio.sleep(interval_s)
            fresh = await self.invocations.get(enqueued["invocation_id"])
            if fresh["status"] in TERMINAL_STATUSES:
                return fresh
        raise TimeoutError(
            f"invoke_and_wait timed out after {timeout_s}s - invocation "
            f"{enqueued['invocation_id']} is still in flight"
        )


class AsyncAgentsClient:
    def __init__(self, chakra: AsyncChakraMCP) -> None:
        self._c = chakra
        self.capabilities = _AsyncCapabilitiesClient(chakra)

    async def list(self) -> list[Agent]:
        return await self._c._relay("GET", "/v1/agents")

    async def get(self, agent_id: str) -> Agent:
        return await self._c._relay("GET", f"/v1/agents/{agent_id}")

    async def create(self, body: CreateAgentRequest | dict[str, Any]) -> Agent:
        return await self._c._relay("POST", "/v1/agents", dict(body))

    async def update(self, agent_id: str, body: UpdateAgentRequest | dict[str, Any]) -> Agent:
        return await self._c._relay("PATCH", f"/v1/agents/{agent_id}", dict(body))

    async def delete(self, agent_id: str) -> None:
        await self._c._relay("DELETE", f"/v1/agents/{agent_id}")


class _AsyncCapabilitiesClient:
    def __init__(self, chakra: AsyncChakraMCP) -> None:
        self._c = chakra

    async def list(self, agent_id: str) -> list[Capability]:
        return await self._c._relay("GET", f"/v1/agents/{agent_id}/capabilities")

    async def create(
        self, agent_id: str, body: CreateCapabilityRequest | dict[str, Any]
    ) -> Capability:
        return await self._c._relay(
            "POST", f"/v1/agents/{agent_id}/capabilities", dict(body)
        )

    async def add_template(
        self,
        agent_id: str,
        template_id: str,
        *,
        description: str | None = None,
        visibility: str | None = None,
    ) -> Capability:
        """Publish a reserved-name capability with its canonical schema.

        ``template_id`` is one of ``chakramcp.template_names()`` (e.g.
        ``"message_owner"``). The input/output schemas come from the
        registry — overriding them defeats the point of having a
        reserved name. Description and visibility *can* be overridden;
        if you don't pass them, the template's defaults apply.

        Returns the same shape as ``create()``.
        """
        from copy import deepcopy

        from ._templates import get_template

        body = deepcopy(get_template(template_id))
        if description is not None:
            body["description"] = description
        if visibility is not None:
            body["visibility"] = visibility
        return await self._c._relay(
            "POST", f"/v1/agents/{agent_id}/capabilities", body
        )

    async def delete(self, agent_id: str, capability_id: str) -> None:
        await self._c._relay(
            "DELETE", f"/v1/agents/{agent_id}/capabilities/{capability_id}"
        )


class AsyncFriendshipsClient:
    def __init__(self, chakra: AsyncChakraMCP) -> None:
        self._c = chakra

    async def list(
        self,
        *,
        direction: str | None = None,
        status: FriendshipStatus | None = None,
    ) -> list[Friendship]:
        params = {k: v for k, v in {"direction": direction, "status": status}.items() if v}
        suffix = "?" + "&".join(f"{k}={v}" for k, v in params.items()) if params else ""
        return await self._c._relay("GET", f"/v1/friendships{suffix}")

    async def get(self, friendship_id: str) -> Friendship:
        return await self._c._relay("GET", f"/v1/friendships/{friendship_id}")

    async def propose(self, body: ProposeFriendshipRequest | dict[str, Any]) -> Friendship:
        return await self._c._relay("POST", "/v1/friendships", dict(body))

    async def accept(self, friendship_id: str, *, message: str | None = None) -> Friendship:
        return await self._c._relay(
            "POST", f"/v1/friendships/{friendship_id}/accept", {"response_message": message}
        )

    async def reject(self, friendship_id: str, *, message: str | None = None) -> Friendship:
        return await self._c._relay(
            "POST", f"/v1/friendships/{friendship_id}/reject", {"response_message": message}
        )

    async def counter(self, friendship_id: str, *, message: str) -> Friendship:
        return await self._c._relay(
            "POST",
            f"/v1/friendships/{friendship_id}/counter",
            {"proposer_message": message},
        )

    async def cancel(self, friendship_id: str) -> Friendship:
        return await self._c._relay("POST", f"/v1/friendships/{friendship_id}/cancel", {})


class AsyncGrantsClient:
    def __init__(self, chakra: AsyncChakraMCP) -> None:
        self._c = chakra

    async def list(
        self,
        *,
        direction: str | None = None,
        status: GrantStatus | None = None,
    ) -> list[Grant]:
        params = {k: v for k, v in {"direction": direction, "status": status}.items() if v}
        suffix = "?" + "&".join(f"{k}={v}" for k, v in params.items()) if params else ""
        return await self._c._relay("GET", f"/v1/grants{suffix}")

    async def get(self, grant_id: str) -> Grant:
        return await self._c._relay("GET", f"/v1/grants/{grant_id}")

    async def create(self, body: CreateGrantRequest | dict[str, Any]) -> Grant:
        return await self._c._relay("POST", "/v1/grants", dict(body))

    async def revoke(self, grant_id: str, *, reason: str | None = None) -> Grant:
        return await self._c._relay(
            "POST", f"/v1/grants/{grant_id}/revoke", {"reason": reason}
        )


class AsyncInvocationsClient:
    def __init__(self, chakra: AsyncChakraMCP) -> None:
        self._c = chakra

    async def list(
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
        return await self._c._relay("GET", f"/v1/invocations{suffix}")

    async def get(self, invocation_id: str) -> Invocation:
        return await self._c._relay("GET", f"/v1/invocations/{invocation_id}")


class AsyncInboxClient:
    def __init__(self, chakra: AsyncChakraMCP) -> None:
        self._c = chakra

    async def pull(self, agent_id: str, *, limit: int | None = None) -> list[Invocation]:
        params: dict[str, str] = {"agent_id": agent_id}
        if limit is not None:
            params["limit"] = str(limit)
        suffix = "&".join(f"{k}={v}" for k, v in params.items())
        return await self._c._relay("GET", f"/v1/inbox?{suffix}")

    async def respond(
        self, invocation_id: str, result: HandlerResult | dict[str, Any]
    ) -> Invocation:
        return await self._c._relay(
            "POST", f"/v1/invocations/{invocation_id}/result", dict(result)
        )

    async def serve(
        self,
        agent_id: str,
        handler: Callable[[Invocation], Awaitable[HandlerResult | dict[str, Any]]],
        *,
        human_handler: Callable[[Invocation], Awaitable[None]] | None = None,
        poll_interval_s: float = 2.0,
        batch_size: int = 25,
        on_error: Callable[[BaseException, Invocation | None], None] | None = None,
        stop_event: asyncio.Event | None = None,
    ) -> None:
        """Long-running pull → dispatch → respond loop.

        ``handler`` is the autonomous path; ``human_handler`` is the
        human-in-the-loop path. When an invocation arrives with
        ``semantics == "human_in_loop"``, the SDK awaits
        ``human_handler(inv)`` and then **moves on without posting a
        result** — the invocation row stays ``in_progress`` until a
        human resolves it out-of-band via
        ``chakramcp message reply <id> "<text>"``. That terminal path
        sets ``confirmed_by_human: true`` on the relay's result
        endpoint, which satisfies the HITL gate.

        ``human_handler`` MUST NOT call ``respond()`` itself. If you
        forget to wire ``human_handler`` while pulling HITL traffic,
        the SDK emits a stderr warning per invocation and leaves the
        row in flight (still resolvable via the CLI reply path).

        Capabilities without ``semantics`` set (or with
        ``"autonomous"``) take the existing path. The field is absent
        on relays older than PR #80 — that's treated as autonomous so
        old workers keep working against new relays and vice versa.

        Pass ``stop_event`` (an :class:`asyncio.Event`) to stop the
        loop cleanly between batches. Handler exceptions are caught
        and reported as failed; cancellation propagates normally so
        ``task.cancel()`` works.
        """
        async def _stopped() -> bool:
            return bool(stop_event and stop_event.is_set())

        while not await _stopped():
            try:
                batch = await self.pull(agent_id, limit=batch_size)
            except BaseException as err:
                if on_error:
                    on_error(err, None)
                await asyncio.sleep(poll_interval_s)
                continue
            if not batch:
                await asyncio.sleep(poll_interval_s)
                continue
            await asyncio.gather(
                *(
                    self._handle_one(inv, handler, human_handler, on_error)
                    for inv in batch
                )
            )

    async def _handle_one(
        self,
        inv: Invocation,
        handler: Callable[[Invocation], Awaitable[HandlerResult | dict[str, Any]]],
        human_handler: Callable[[Invocation], Awaitable[None]] | None,
        on_error: Callable[[BaseException, Invocation | None], None] | None,
    ) -> None:
        # HITL routing — defer to human_handler and skip respond().
        # The row stays in_progress; a human resolves it later via
        # the CLI reply path which sets `confirmed_by_human: true`
        # on the result.
        if inv.get("semantics") == "human_in_loop":
            if human_handler is not None:
                try:
                    await human_handler(inv)
                except BaseException as err:
                    if on_error:
                        on_error(err, inv)
            else:
                import sys

                print(
                    f"[chakramcp] warning: pulled human_in_loop "
                    f"invocation {inv['id']} "
                    f"({inv.get('capability_name', '?')}) but no "
                    f"human_handler is set. Leaving the row "
                    f"in_progress; the human can reply via "
                    f"`chakramcp message reply {inv['id']} "
                    f'"<text>"`.',
                    file=sys.stderr,
                )
            return
        try:
            result = await handler(inv)
            await self.respond(inv["id"], result)
        except BaseException as err:
            if on_error:
                on_error(err, inv)
            try:
                await self.respond(inv["id"], {"status": "failed", "error": str(err)})
            except BaseException as inner:
                if on_error:
                    on_error(inner, inv)
