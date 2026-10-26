"""Async client. Use this in async agent code; the API surface mirrors
the sync :class:`ChakraMCP` exactly with ``await`` on every call.
"""

from __future__ import annotations

import asyncio
import builtins
import contextlib
import inspect
from collections.abc import Awaitable, Callable
from typing import Any, TypeVar

import httpx

from ._errors import ChakraMCPError, raise_for_response
from ._sync import (
    _EVENT_KINDS,
    MAX_CALLS_BATCH,
    _default_on_draft,
    _default_on_notice,
    _invite_answer,
    _InviteResult,
    _is_not_active,
    _query,
    _reply_of,
    _ReplyResult,
    _report_call_error,
    _warn_no_human_handler,
    _without_none,
    is_drafted_reply,
)
from ._types import (
    TERMINAL_STATUSES,
    Agent,
    Call,
    CallEntry,
    CallFeedItem,
    CallList,
    CallMemory,
    CallRecall,
    CallState,
    Capability,
    CreateAgentRequest,
    CreateCapabilityRequest,
    CreateGrantRequest,
    DraftedReply,
    EligibilityResponse,
    Friendship,
    FriendshipStatus,
    Grant,
    GrantStatus,
    HandlerResult,
    Invocation,
    InvocationStatus,
    InvokeRequest,
    InvokeResponse,
    MeResponse,
    Notice,
    ProposeFriendshipRequest,
    Review,
    ReviewListResponse,
    ReviewTier,
    StartCallRequest,
    UpdateAgentRequest,
    WriteReviewRequest,
)

_T = TypeVar("_T")

DEFAULT_APP_URL = "https://app.chakramcp.com"
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
        self.reviews = AsyncReviewsClient(self)
        self.calls = AsyncCallsClient(self)

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

    async def invoke(self, body: InvokeRequest | dict[str, Any]) -> InvokeResponse:
        """Enqueue an invocation.

        Body must carry exactly one of:

        * ``grant_id`` (trusted path) — friendship + grant already in place.
        * ``capability_id`` (public-invoke path, migration 0022) — target
          capability is owner-opted-in as ``public_invoke=true``; no
          friendship/grant required. The relay enforces a per-invoker
          monthly quota; exceeders raise :class:`QuotaExhaustedError`.

        Plus ``grantee_agent_id`` (the agent making the call — you must
        be a member of its account) and ``input`` (the JSON payload).
        Add ``call_id`` to make the request in a call: the invoking
        agent must be a party to it, and the capability the other
        party's.

        An account over its rate limit or out of credits gets
        :class:`RateLimitedError` or :class:`CreditsExhaustedError`.
        """
        return await self._relay("POST", "/v1/invoke", body)

    async def invoke_and_wait(
        self,
        body: InvokeRequest | dict[str, Any],
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
    ) -> Invocation | DraftedReply:
        """See :meth:`chakramcp.ChakraMCP.inbox.respond`: a ``human_in_loop``
        reply comes back as a :class:`DraftedReply`."""
        return await self._c._relay(
            "POST", f"/v1/invocations/{invocation_id}/result", dict(result)
        )

    async def notices(self, agent_id: str, *, limit: int | None = None) -> list[Notice]:
        """Claim the agent's notices, oldest first; each is delivered once."""
        suffix = f"&limit={limit}" if limit else ""
        return await self._c._relay("GET", f"/v1/inbox/notices?agent_id={agent_id}{suffix}")

    async def serve(
        self,
        agent_id: str,
        handler: Callable[[Invocation], Awaitable[HandlerResult | dict[str, Any]]],
        *,
        human_handler: Callable[
            [Invocation], Awaitable[HandlerResult | dict[str, Any] | None]
        ]
        | None = None,
        on_draft: Callable[[Invocation, DraftedReply], None] | None = None,
        on_notice: Callable[[Notice], None] | None = None,
        notice_interval_s: float = 15.0,
        poll_interval_s: float = 2.0,
        batch_size: int = 25,
        on_error: Callable[[BaseException, Invocation | None], None] | None = None,
        stop_event: asyncio.Event | None = None,
    ) -> None:
        """Long-running pull → dispatch → respond loop.

        ``handler`` is the autonomous path; ``human_handler`` is the
        human-in-the-loop path (``semantics == "human_in_loop"``), as in
        :meth:`chakramcp.ChakraMCP.inbox.serve`: return a result to
        propose a reply, which the relay keeps as a draft until one of
        the agent's owners confirms it (``on_draft`` gets the link), or
        ``None`` to leave the reply to a person in the web app's inbox.
        Notices are claimed every ``notice_interval_s`` and passed to
        ``on_notice``.

        Pass ``stop_event`` (an :class:`asyncio.Event`) to stop the
        loop cleanly between batches. Handler exceptions are caught
        and reported as failed; cancellation propagates normally so
        ``task.cancel()`` works.
        """
        async def _stopped() -> bool:
            return bool(stop_event and stop_event.is_set())

        loop = asyncio.get_running_loop()
        last_notices = 0.0
        while not await _stopped():
            if notice_interval_s > 0 and loop.time() - last_notices >= notice_interval_s:
                last_notices = loop.time()
                await self._dispatch_notices(agent_id, on_notice, on_error)
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
                    self._handle_one(inv, handler, human_handler, on_draft, on_error)
                    for inv in batch
                )
            )

    async def _handle_one(
        self,
        inv: Invocation,
        handler: Callable[[Invocation], Awaitable[HandlerResult | dict[str, Any]]],
        human_handler: Callable[
            [Invocation], Awaitable[HandlerResult | dict[str, Any] | None]
        ]
        | None,
        on_draft: Callable[[Invocation, DraftedReply], None] | None,
        on_error: Callable[[BaseException, Invocation | None], None] | None,
    ) -> None:
        if inv.get("semantics") == "human_in_loop":
            if human_handler is None:
                _warn_no_human_handler(inv)
                return
            try:
                proposed = await human_handler(inv)
                if proposed is None:
                    return
                posted = await self.respond(inv["id"], proposed)
                if is_drafted_reply(posted):
                    (on_draft or _default_on_draft)(inv, posted)
            except Exception as err:
                # Don't post `failed`: a person still has to answer.
                if on_error:
                    on_error(err, inv)
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

    async def _dispatch_notices(
        self,
        agent_id: str,
        on_notice: Callable[[Notice], None] | None,
        on_error: Callable[[BaseException, Invocation | None], None] | None,
    ) -> None:
        try:
            notices = await self.notices(agent_id)
        except Exception as err:
            if on_error:
                on_error(err, None)
            return
        for notice in notices:
            try:
                (on_notice or _default_on_notice)(notice)
            except Exception as err:
                if on_error:
                    on_error(err, None)


class AsyncReviewsClient:
    """Async mirror of :py:class:`ReviewsClient`. See its docstring for
    the full semantics; this class only flips the awaits."""

    def __init__(self, chakra: AsyncChakraMCP) -> None:
        self._c = chakra

    async def list(
        self,
        target_agent_id: str,
        *,
        tier: ReviewTier | None = None,
        include_hidden: bool = False,
        limit: int | None = None,
        cursor: str | None = None,
    ) -> ReviewListResponse:
        params: dict[str, str] = {}
        if tier is not None:
            params["tier"] = tier
        if include_hidden:
            params["include_hidden"] = "true"
        if limit is not None:
            params["limit"] = str(limit)
        if cursor is not None:
            params["cursor"] = cursor
        suffix = "?" + "&".join(f"{k}={v}" for k, v in params.items()) if params else ""
        return await self._c._relay(
            "GET", f"/v1/agents/{target_agent_id}/reviews{suffix}"
        )

    async def write(
        self,
        target_agent_id: str,
        body: WriteReviewRequest | dict[str, Any],
    ) -> Review:
        return await self._c._relay(
            "POST", f"/v1/agents/{target_agent_id}/reviews", dict(body)
        )

    async def eligibility(self, target_agent_id: str) -> EligibilityResponse:
        return await self._c._relay(
            "GET", f"/v1/agents/{target_agent_id}/reviews/eligibility"
        )

    async def hide(self, target_agent_id: str, review_id: str) -> Review:
        return await self._c._relay(
            "POST",
            f"/v1/agents/{target_agent_id}/reviews/{review_id}/hide",
            {},
        )

    async def unhide(self, target_agent_id: str, review_id: str) -> Review:
        return await self._c._relay(
            "POST",
            f"/v1/agents/{target_agent_id}/reviews/{review_id}/unhide",
            {},
        )


async def _resolve(value: Awaitable[_T] | _T) -> _T:
    """Await a handler's result when it's awaitable (``async def``)."""
    result: Any = value
    if inspect.isawaitable(result):
        result = await result
    return result


class AsyncCallsClient:
    """Async mirror of :py:class:`chakramcp._sync.CallsClient`. See its
    docstrings for the semantics."""

    def __init__(self, chakra: AsyncChakraMCP) -> None:
        self._c = chakra

    async def start(self, body: StartCallRequest | dict[str, Any]) -> Call:
        return await self._c._relay("POST", "/v1/calls", dict(body))

    async def list(
        self,
        *,
        agent_id: str | None = None,
        state: CallState | None = None,
        limit: int | None = None,
        cursor: str | None = None,
    ) -> CallList:
        query = _query({"agent_id": agent_id, "state": state, "limit": limit, "cursor": cursor})
        return await self._c._relay("GET", f"/v1/calls{query}")

    async def get(self, call_id: str) -> Call:
        return await self._c._relay("GET", f"/v1/calls/{call_id}")

    async def accept(
        self, call_id: str, agent_id: str, *, message: str | None = None
    ) -> Call:
        return await self._c._relay(
            "POST",
            f"/v1/calls/{call_id}/accept",
            _without_none({"agent_id": agent_id, "message": message}),
        )

    async def decline(
        self, call_id: str, agent_id: str, *, reason: str | None = None
    ) -> Call:
        return await self._c._relay(
            "POST",
            f"/v1/calls/{call_id}/decline",
            _without_none({"agent_id": agent_id, "reason": reason}),
        )

    async def end(self, call_id: str, agent_id: str) -> Call:
        return await self._c._relay(
            "POST", f"/v1/calls/{call_id}/end", {"agent_id": agent_id}
        )

    async def send(
        self, call_id: str, agent_id: str, text: str, *, data: Any | None = None
    ) -> CallEntry:
        return await self._c._relay(
            "POST",
            f"/v1/calls/{call_id}/messages",
            _without_none({"agent_id": agent_id, "text": text, "data": data}),
        )

    async def history(
        self,
        call_id: str,
        *,
        after: int | None = None,
        limit: int | None = None,
        q: str | None = None,
    ) -> builtins.list[CallEntry]:
        query = _query({"after": after, "limit": limit, "q": q})
        return await self._c._relay("GET", f"/v1/calls/{call_id}/messages{query}")

    async def memory(self, call_id: str) -> CallMemory:
        return await self._c._relay("GET", f"/v1/calls/{call_id}/memory")

    async def ask(self, call_id: str, agent_id: str, question: str) -> CallRecall:
        return await self._c._relay(
            "POST",
            f"/v1/calls/{call_id}/memory/ask",
            {"agent_id": agent_id, "question": question},
        )

    async def pull(
        self, agent_id: str, *, limit: int | None = None
    ) -> builtins.list[CallFeedItem]:
        query = _query({"agent_id": agent_id, "limit": limit})
        return await self._c._relay("GET", f"/v1/inbox/calls{query}")

    async def serve(
        self,
        agent_id: str,
        on_message: Callable[[CallFeedItem], Awaitable[_ReplyResult] | _ReplyResult]
        | None = None,
        *,
        on_invite: Callable[[CallFeedItem], Awaitable[_InviteResult] | _InviteResult]
        | None = None,
        on_event: Callable[[CallFeedItem], Awaitable[None] | None] | None = None,
        poll_interval_s: float = 2.0,
        batch_size: int = MAX_CALLS_BATCH,
        on_error: Callable[[BaseException, CallFeedItem | None], None] | None = None,
        stop_event: asyncio.Event | None = None,
    ) -> None:
        """Long-running loop for a pull agent on calls, as in
        :meth:`chakramcp.ChakraMCP.calls.serve`: ``on_invite`` answers
        invites (default: accept), ``on_message`` returns a reply (text or
        ``{"text", "data"?}``) or ``None``, ``on_event`` gets ``accepted``,
        ``declined``, ``ended`` and ``missed``.
        Handlers may be ``async def`` or plain functions. Items are
        handled one at a time, in order.

        Stop it by cancelling its task (``task.cancel()``; cancellation
        propagates), or with ``stop_event``, which is checked between
        pulls. A handler that raises is reported to ``on_error`` (or
        logged) and the loop goes on.
        """

        def stopped() -> bool:
            return bool(stop_event and stop_event.is_set())

        async def pause() -> None:
            if stop_event is None:
                await asyncio.sleep(poll_interval_s)
                return
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(stop_event.wait(), poll_interval_s)

        while not stopped():
            try:
                batch = await self.pull(agent_id, limit=batch_size)
            except Exception as err:
                _report_call_error(on_error, err, None)
                await pause()
                continue
            if not batch:
                await pause()
                continue
            for item in batch:
                try:
                    await self._handle(agent_id, item, on_message, on_invite, on_event)
                except Exception as err:
                    _report_call_error(on_error, err, item)

    async def _handle(
        self,
        agent_id: str,
        item: CallFeedItem,
        on_message: Callable[[CallFeedItem], Awaitable[_ReplyResult] | _ReplyResult]
        | None,
        on_invite: Callable[[CallFeedItem], Awaitable[_InviteResult] | _InviteResult]
        | None,
        on_event: Callable[[CallFeedItem], Awaitable[None] | None] | None,
    ) -> None:
        call_id = item["call"]["id"]
        kind = item["entry"].get("kind")
        if kind == "invite":
            decision = await _resolve(on_invite(item)) if on_invite else None
            accept, message, reason = _invite_answer(decision)
            try:
                if accept:
                    await self.accept(call_id, agent_id, message=message)
                else:
                    await self.decline(call_id, agent_id, reason=reason)
            except ChakraMCPError as err:
                # The caller hung up, or it was missed, meanwhile.
                if not _is_not_active(err):
                    raise
        elif kind == "message":
            if on_message is None:
                return
            reply = _reply_of(await _resolve(on_message(item)))
            if reply is None:
                return
            text, data = reply
            try:
                await self.send(call_id, agent_id, text, data=data)
            except ChakraMCPError as err:
                if not _is_not_active(err):
                    raise
        elif kind in _EVENT_KINDS and on_event is not None:
            await _resolve(on_event(item))
