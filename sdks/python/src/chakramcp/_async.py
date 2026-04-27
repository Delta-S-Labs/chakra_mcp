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
            f"invoke_and_wait timed out after {timeout_s}s — invocation "
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
        poll_interval_s: float = 2.0,
        batch_size: int = 25,
        on_error: Callable[[BaseException, Invocation | None], None] | None = None,
        stop_event: asyncio.Event | None = None,
    ) -> None:
        """Long-running pull → await handler → respond loop.

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
            await asyncio.gather(*(self._handle_one(inv, handler, on_error) for inv in batch))

    async def _handle_one(
        self,
        inv: Invocation,
        handler: Callable[[Invocation], Awaitable[HandlerResult | dict[str, Any]]],
        on_error: Callable[[BaseException, Invocation | None], None] | None,
    ) -> None:
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
