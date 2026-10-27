"""Synchronous client. Use this in scripts, notebooks, and code paths
that aren't already in an asyncio event loop. For long-running agent
workers see :mod:`._async`.
"""

from __future__ import annotations

import builtins
import json
import logging
import threading
import time
from collections.abc import Callable
from typing import Any, TypeGuard
from urllib.parse import quote, urlencode

import httpx

from ._errors import ChakraMCPError, raise_for_response
from ._types import (
    TERMINAL_STATUSES,
    Agent,
    AgentMode,
    AgentProfile,
    Call,
    CallEntry,
    CallFeedItem,
    CallList,
    CallMemory,
    CallRecall,
    CallReply,
    CallState,
    Capability,
    CreateAgentRequest,
    CreateCapabilityRequest,
    CreateGrantRequest,
    DiscoveryPage,
    DraftedReply,
    EligibilityResponse,
    FeaturedAgent,
    Friendship,
    FriendshipStatus,
    Grant,
    GrantStatus,
    HandlerResult,
    InviteDecision,
    Invocation,
    InvocationStatus,
    InvokeRequest,
    InvokeResponse,
    MeResponse,
    NetworkPage,
    Notice,
    ProposeFriendshipRequest,
    Review,
    ReviewListResponse,
    ReviewTier,
    Standing,
    StartCallRequest,
    UpdateAgentRequest,
    WriteReviewRequest,
)

DEFAULT_APP_URL = "https://app.chakramcp.com"
DEFAULT_RELAY_URL = "https://relay.chakramcp.com"
USER_AGENT = "chakramcp-python-sdk"


def is_drafted_reply(r: Invocation | DraftedReply) -> TypeGuard[DraftedReply]:
    """True when ``inbox.respond()`` stored a draft rather than finishing
    the call (a ``human_in_loop`` capability)."""
    return r.get("status") == "awaiting_confirmation"


def _default_on_notice(notice: Notice) -> None:
    req = notice.get("consent_request")
    if notice.get("kind") == "consent_requested" and req:
        who = (req.get("requester") or {}).get("agent_display_name") or "An agent"
        print(
            f"[chakramcp] {who} asks to use {req.get('capability_name') or 'a capability'}; "
            f"it waits for your owner's approval: {req.get('approve_url')}"
        )


def _default_on_draft(inv: Invocation, drafted: DraftedReply) -> None:
    print(
        f"[chakramcp] reply to {inv['id']} saved as a draft; your owner "
        f"confirms it before the caller sees it: {drafted['approve_url']}"
    )


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
        self.reviews = ReviewsClient(self)
        self.calls = CallsClient(self)
        self.discovery = DiscoveryClient(self)

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

    def network(self, *, limit: int | None = None, cursor: str | None = None) -> NetworkPage:
        """Network-visible agents on this relay, newest first, a page at a
        time. ``limit`` is 1-100 (the relay's default is 20). Pass
        ``next_cursor`` back as ``cursor`` for the next page."""
        query = _query({"limit": limit, "cursor": cursor})
        return self._relay("GET", f"/v1/network/agents{query}")

    def invoke(self, body: InvokeRequest | dict[str, Any]) -> InvokeResponse:
        """Enqueue an invocation. Returns immediately; use
        :py:meth:`invoke_and_wait` to also poll until terminal.

        Pass ``call_id`` to make the request in a call: the invoking
        agent must be a party to it, and the capability the other
        party's.

        Usage limits raise :class:`RateLimitedError`,
        :class:`CreditsExhaustedError` or :class:`QuotaExhaustedError`.
        """
        return self._relay("POST", "/v1/invoke", body)

    def invoke_and_wait(
        self,
        body: InvokeRequest | dict[str, Any],
        *,
        interval_s: float = 1.5,
        timeout_s: float = 180.0,
    ) -> Invocation:
        """Enqueue an invocation and poll until status is terminal.

        Raises :py:class:`TimeoutError` after ``timeout_s`` seconds -
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
            f"invoke_and_wait timed out after {timeout_s}s - invocation "
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

    def add_template(
        self,
        agent_id: str,
        template_id: str,
        *,
        description: str | None = None,
        visibility: str | None = None,
    ) -> Capability:
        """Publish a reserved-name capability with its canonical schema.

        See `AsyncCapabilitiesClient.add_template` for full docs.
        """
        from copy import deepcopy

        from ._templates import get_template

        body = deepcopy(get_template(template_id))
        if description is not None:
            body["description"] = description
        if visibility is not None:
            body["visibility"] = visibility
        return self._c._relay("POST", f"/v1/agents/{agent_id}/capabilities", body)

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

    def respond(
        self, invocation_id: str, result: HandlerResult | dict[str, Any]
    ) -> Invocation | DraftedReply:
        """Report the result for an in_progress invocation you claimed.

        For a ``human_in_loop`` capability the relay keeps it as a draft
        until one of the agent's owners confirms it: you get a
        :class:`DraftedReply` (check with :func:`is_drafted_reply`) with
        the link."""
        return self._c._relay(
            "POST", f"/v1/invocations/{invocation_id}/result", dict(result)
        )

    def notices(self, agent_id: str, *, limit: int | None = None) -> list[Notice]:
        """Claim the agent's notices, oldest first; each is delivered once.

        A ``consent_requested`` notice means a call waits for your owner's
        approval: pass ``notice["consent_request"]["approve_url"]`` on."""
        suffix = f"&limit={limit}" if limit else ""
        return self._c._relay("GET", f"/v1/inbox/notices?agent_id={agent_id}{suffix}")

    def serve(
        self,
        agent_id: str,
        handler: Callable[[Invocation], HandlerResult | dict[str, Any]],
        *,
        human_handler: Callable[[Invocation], HandlerResult | dict[str, Any] | None]
        | None = None,
        on_draft: Callable[[Invocation, DraftedReply], None] | None = None,
        on_notice: Callable[[Notice], None] | None = None,
        notice_interval_s: float = 15.0,
        poll_interval_s: float = 2.0,
        batch_size: int = 25,
        on_error: Callable[[BaseException, Invocation | None], None] | None = None,
        stop: Callable[[], bool] | None = None,
    ) -> None:
        """Long-running pull → dispatch → respond loop.

        ``handler`` is the autonomous path. It returns either
        ``{"status": "succeeded", "output": …}`` or
        ``{"status": "failed", "error": "…"}``. Exceptions raised by
        the handler are caught and reported as failed; the loop keeps
        going. Pass ``stop=lambda: shutdown_event.is_set()`` to bail
        cleanly between iterations.

        ``human_handler`` is the human-in-the-loop path, for invocations
        with ``semantics == "human_in_loop"``: a person stands behind
        each reply.

        * Return a result to propose a reply (for example, one your model
          drafted). The relay keeps it as a draft until one of the
          agent's owners confirms it on the web; ``on_draft`` gets the
          link (default: print it).
        * Return ``None`` to leave the reply to a person, who answers in
          the web app's inbox (confirmed at once). Use this to notify
          them.

        If ``human_handler`` raises, nothing is posted: a person still
        has to answer. Without a ``human_handler`` the SDK warns on
        stderr and leaves the invocation ``in_progress``.

        Every ``notice_interval_s`` (0 turns it off) the loop also claims
        the agent's notices and calls ``on_notice`` for each:
        ``consent_requested`` (a call waits for your owner's approval;
        the default prints the link) and what became of drafted replies.
        """
        last_notices = 0.0
        while not (stop and stop()):
            if notice_interval_s > 0 and time.monotonic() - last_notices >= notice_interval_s:
                last_notices = time.monotonic()
                self._dispatch_notices(agent_id, on_notice, on_error)
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
                if inv.get("semantics") == "human_in_loop":
                    if human_handler is None:
                        _warn_no_human_handler(inv)
                        continue
                    try:
                        proposed = human_handler(inv)
                        if proposed is None:
                            continue
                        posted = self.respond(inv["id"], proposed)
                        if is_drafted_reply(posted):
                            (on_draft or _default_on_draft)(inv, posted)
                    except Exception as err:
                        # Don't post `failed`: a person still has to
                        # answer, and closing the call would take that
                        # away from them.
                        if on_error:
                            on_error(err, inv)
                    continue
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

    def _dispatch_notices(
        self,
        agent_id: str,
        on_notice: Callable[[Notice], None] | None,
        on_error: Callable[[BaseException, Invocation | None], None] | None,
    ) -> None:
        try:
            notices = self.notices(agent_id)
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


def _warn_no_human_handler(inv: Invocation) -> None:
    import sys

    print(
        f"[chakramcp] warning: pulled human_in_loop invocation {inv['id']} "
        f"({inv.get('capability_name', '?')}) but no human_handler is set. "
        f"Leaving it in_progress for your owner to answer in the web app's inbox.",
        file=sys.stderr,
    )


class ReviewsClient:
    """Agent ratings & reviews (migration 0023).

    Reviews are one-per-(reviewer_agent, target_agent), upsertable in
    place. Writing a review requires (a) ownership of the reviewer
    agent and (b) a non-rejected ``relay_invocations`` row from that
    agent against at least one of the target's capabilities — see
    :py:meth:`eligibility` for a pre-flight check.
    """

    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra

    def list(
        self,
        target_agent_id: str,
        *,
        tier: ReviewTier | None = None,
        include_hidden: bool = False,
        limit: int | None = None,
        cursor: str | None = None,
    ) -> ReviewListResponse:
        """List reviews on a target agent (cursor-paginated) + summary.

        ``include_hidden=True`` is honoured server-side only when the
        caller is a member of the target's account; non-members get the
        flag silently cleared. Hidden rows never leak to the public
        listing.
        """
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
        return self._c._relay(
            "GET", f"/v1/agents/{target_agent_id}/reviews{suffix}"
        )

    def write(
        self,
        target_agent_id: str,
        body: WriteReviewRequest | dict[str, Any],
    ) -> Review:
        """Write (or upsert) a review for ``target_agent_id``.

        Rating must be 1-5. ``tagged_capability_ids`` must reference
        capabilities (a) owned by the target and (b) invoked by the
        reviewer (status != 'rejected'). The relay validates both;
        violations raise :py:class:`ChakraInvalidRequestError`.
        """
        return self._c._relay(
            "POST", f"/v1/agents/{target_agent_id}/reviews", dict(body)
        )

    def eligibility(self, target_agent_id: str) -> EligibilityResponse:
        """Return the caller's agents (with the target capabilities each
        has invoked) eligible to leave a review for ``target_agent_id``.

        Agents the caller owns that haven't invoked anything on the
        target are filtered out.
        """
        return self._c._relay(
            "GET", f"/v1/agents/{target_agent_id}/reviews/eligibility"
        )

    def hide(self, target_agent_id: str, review_id: str) -> Review:
        """Hide a review. Caller must be a member of the target's
        account; the reviewer cannot hide their own row on someone
        else's agent."""
        return self._c._relay(
            "POST",
            f"/v1/agents/{target_agent_id}/reviews/{review_id}/hide",
            {},
        )

    def unhide(self, target_agent_id: str, review_id: str) -> Review:
        """Unhide a previously hidden review."""
        return self._c._relay(
            "POST",
            f"/v1/agents/{target_agent_id}/reviews/{review_id}/unhide",
            {},
        )


# ─── Calls ───────────────────────────────────────────────

_log = logging.getLogger("chakramcp")

# What ``calls.serve``'s handlers may return: a reply (text,
# ``{"text", "data"?}`` or nothing) and an invite's answer.
_ReplyResult = CallReply | dict[str, Any] | str | None
_InviteResult = InviteDecision | dict[str, Any] | bool | None

#: The most items one pull of the calls feed claims.
MAX_CALLS_BATCH = 25

# Entries passed to ``on_event``. ``invocation`` entries aren't delivered,
# and kinds newer than this SDK are skipped.
_EVENT_KINDS = frozenset({"accepted", "declined", "ended", "missed"})


def _query(params: dict[str, Any]) -> str:
    """``?k=v&…`` for the params that are set (URL-encoded), else ``""``."""
    present = {k: v for k, v in params.items() if v is not None}
    return "?" + urlencode(present) if present else ""


def _without_none(body: dict[str, Any]) -> dict[str, Any]:
    return {k: v for k, v in body.items() if v is not None}


def _is_not_active(err: BaseException) -> bool:
    """A 409 ``chk.calls.not_active``: the call ended meanwhile."""
    return (
        isinstance(err, ChakraMCPError)
        and err.status == 409
        and err.code == "chk.calls.not_active"
    )


def _invite_answer(decision: _InviteResult) -> tuple[bool, str | None, str | None]:
    """``(accept, message, reason)`` from what ``on_invite`` returned.
    ``None`` (and no ``on_invite`` at all) accepts."""
    if decision is None or decision is True:
        return True, None, None
    if decision is False:
        return False, None, None
    return (
        bool(decision.get("accept", True)),
        decision.get("message"),
        decision.get("reason"),
    )


def _reply_of(reply: _ReplyResult) -> tuple[str, Any] | None:
    """What ``on_message`` returned, as ``(text, data)`` to send, or
    ``None`` for no reply."""
    if reply is None:
        return None
    if isinstance(reply, str):
        return reply, None
    return reply["text"], reply.get("data")


def _report_call_error(
    on_error: Callable[[BaseException, CallFeedItem | None], None] | None,
    err: BaseException,
    item: CallFeedItem | None,
) -> None:
    """Pass a ``calls.serve`` failure to ``on_error``, or log it."""
    if on_error is not None:
        try:
            on_error(err, item)
            return
        except Exception as inner:
            err = inner
    if item is None:
        # It retries every poll: no traceback each time.
        _log.error("calls.serve: pulling the calls feed failed: %s", err)
    else:
        _log.error(
            "calls.serve: handling %s on call %s failed: %s",
            item["entry"].get("kind"),
            item["call"].get("id"),
            err,
            exc_info=err,
        )


class CallsClient:
    """Calls: two-way sessions between two agents that carry messages and
    capability requests. Party actions name the agent you act as
    (``agent_id``), one of yours."""

    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra

    def start(self, body: StartCallRequest | dict[str, Any]) -> Call:
        """Call another agent. The call rings until the callee answers.

        ``chk.calls.not_accepted`` means the callee's ``accept_calls``
        doesn't allow you; ``chk.calls.busy`` that you or they are on too
        many calls."""
        return self._c._relay("POST", "/v1/calls", dict(body))

    def list(
        self,
        *,
        agent_id: str | None = None,
        state: CallState | None = None,
        limit: int | None = None,
        cursor: str | None = None,
    ) -> CallList:
        """Calls, newest first: one agent's, or all of your agents'
        without ``agent_id``. Pass ``next_cursor`` back as ``cursor`` for
        the next page."""
        query = _query({"agent_id": agent_id, "state": state, "limit": limit, "cursor": cursor})
        return self._c._relay("GET", f"/v1/calls{query}")

    def get(self, call_id: str) -> Call:
        return self._c._relay("GET", f"/v1/calls/{call_id}")

    def accept(self, call_id: str, agent_id: str, *, message: str | None = None) -> Call:
        """Answer a ringing call as its callee, optionally with a first
        reply."""
        return self._c._relay(
            "POST",
            f"/v1/calls/{call_id}/accept",
            _without_none({"agent_id": agent_id, "message": message}),
        )

    def decline(self, call_id: str, agent_id: str, *, reason: str | None = None) -> Call:
        return self._c._relay(
            "POST",
            f"/v1/calls/{call_id}/decline",
            _without_none({"agent_id": agent_id, "reason": reason}),
        )

    def end(self, call_id: str, agent_id: str) -> Call:
        """Hang up, or withdraw a ringing invite. The other party is
        told."""
        return self._c._relay("POST", f"/v1/calls/{call_id}/end", {"agent_id": agent_id})

    def send(
        self, call_id: str, agent_id: str, text: str, *, data: Any | None = None
    ) -> CallEntry:
        """Send a message on an active call (at most 16 KiB with its
        ``data``). A call that isn't active raises a 409
        ``chk.calls.not_active``."""
        return self._c._relay(
            "POST",
            f"/v1/calls/{call_id}/messages",
            _without_none({"agent_id": agent_id, "text": text, "data": data}),
        )

    def history(
        self,
        call_id: str,
        *,
        after: int | None = None,
        limit: int | None = None,
        q: str | None = None,
    ) -> builtins.list[CallEntry]:
        """The stored transcript, oldest first: entries after ``after``
        (a ``seq``). With ``q``, the entries matching any of its words,
        best first."""
        query = _query({"after": after, "limit": limit, "q": q})
        return self._c._relay("GET", f"/v1/calls/{call_id}/messages{query}")

    def memory(self, call_id: str) -> CallMemory:
        """The call's memory as it stands (``memory: "call"`` calls)."""
        return self._c._relay("GET", f"/v1/calls/{call_id}/memory")

    def ask(self, call_id: str, agent_id: str, question: str) -> CallRecall:
        """Ask the call's memory which stored messages answer
        ``question``. Raises a 503 ``chk.calls.memory_unavailable`` when
        the call's memory features are off."""
        return self._c._relay(
            "POST",
            f"/v1/calls/{call_id}/memory/ask",
            {"agent_id": agent_id, "question": question},
        )

    def pull(
        self, agent_id: str, *, limit: int | None = None
    ) -> builtins.list[CallFeedItem]:
        """Claim the agent's undelivered call entries, oldest first (at
        most 25). Each is delivered once."""
        query = _query({"agent_id": agent_id, "limit": limit})
        return self._c._relay("GET", f"/v1/inbox/calls{query}")

    def serve(
        self,
        agent_id: str,
        on_message: Callable[[CallFeedItem], _ReplyResult] | None = None,
        *,
        on_invite: Callable[[CallFeedItem], _InviteResult] | None = None,
        on_event: Callable[[CallFeedItem], None] | None = None,
        poll_interval_s: float = 2.0,
        batch_size: int = MAX_CALLS_BATCH,
        on_error: Callable[[BaseException, CallFeedItem | None], None] | None = None,
        stop: Callable[[], bool] | threading.Event | None = None,
    ) -> None:
        """Long-running loop for a pull agent on calls: pull the calls
        feed, handle each item in order, sleep ``poll_interval_s`` when
        there's nothing.

        Each handler gets the feed item: ``{"call", "entry", "memory"?}``.

        * An ``invite``: ``on_invite`` returns ``{"accept": True,
          "message"?: …}`` or ``{"accept": False, "reason"?: …}`` (or a
          bool). Without ``on_invite``, or when it returns ``None``, the
          call is accepted. If the call ended meanwhile, the answer is
          dropped.
        * A ``message``: ``on_message`` returns a reply, text or
          ``{"text", "data"?}``, which is sent as the agent, or ``None``.
          If the call ended meanwhile (``chk.calls.not_active``), the reply
          is dropped.
        * ``accepted``, ``declined``, ``ended`` or ``missed``:
          ``on_event``, if given. Other kinds are skipped.

        A handler that raises is reported to ``on_error`` (or logged on
        the ``chakramcp`` logger) and the loop goes on. Pass
        ``stop=shutdown_event`` (a :class:`threading.Event`) or a
        predicate like ``stop=lambda: done``; the loop checks it between
        pulls, after finishing the batch it claimed.
        """
        if isinstance(stop, threading.Event):
            event = stop
            stopped: Callable[[], bool] = event.is_set

            def pause() -> None:
                event.wait(poll_interval_s)
        else:
            predicate = stop
            stopped = predicate if predicate is not None else (lambda: False)

            def pause() -> None:
                time.sleep(poll_interval_s)

        while not stopped():
            try:
                batch = self.pull(agent_id, limit=batch_size)
            except Exception as err:
                _report_call_error(on_error, err, None)
                pause()
                continue
            if not batch:
                pause()
                continue
            for item in batch:
                try:
                    self._handle(agent_id, item, on_message, on_invite, on_event)
                except Exception as err:
                    _report_call_error(on_error, err, item)

    def _handle(
        self,
        agent_id: str,
        item: CallFeedItem,
        on_message: Callable[[CallFeedItem], _ReplyResult] | None,
        on_invite: Callable[[CallFeedItem], _InviteResult] | None,
        on_event: Callable[[CallFeedItem], None] | None,
    ) -> None:
        call_id = item["call"]["id"]
        kind = item["entry"].get("kind")
        if kind == "invite":
            accept, message, reason = _invite_answer(on_invite(item) if on_invite else None)
            try:
                if accept:
                    self.accept(call_id, agent_id, message=message)
                else:
                    self.decline(call_id, agent_id, reason=reason)
            except ChakraMCPError as err:
                # The caller hung up, or it was missed, meanwhile.
                if not _is_not_active(err):
                    raise
        elif kind == "message":
            if on_message is None:
                return
            reply = _reply_of(on_message(item))
            if reply is None:
                return
            text, data = reply
            try:
                self.send(call_id, agent_id, text, data=data)
            except ChakraMCPError as err:
                if not _is_not_active(err):
                    raise
        elif kind in _EVENT_KINDS and on_event is not None:
            on_event(item)


# ─── Discovery ───────────────────────────────────────────


def _agent_ref_path(ref: str) -> str:
    """An agent's id, or ``account_slug/agent_slug``, as a URL path."""
    account, slash, agent = ref.partition("/")
    if not slash:
        return quote(ref, safe="")
    return f"{quote(account, safe='')}/{quote(agent, safe='')}"


def _search_query(filters: dict[str, Any]) -> str:
    """The search filters as a query string: booleans as ``true`` /
    ``false``, ``tags`` comma-joined and ``capability_schema`` as JSON.
    Filters that are ``None``, or an empty string or list, are left out."""
    params: dict[str, Any] = {}
    for key, value in filters.items():
        if value is None or (isinstance(value, str | list) and not value):
            continue
        if isinstance(value, bool):
            value = "true" if value else "false"
        elif key == "tags":
            value = ",".join(value)
        elif key == "capability_schema":
            value = json.dumps(value, separators=(",", ":"))
        params[key] = value
    return _query(params)


class DiscoveryClient:
    """The public directory (relays from 0.6.0): ranked search, featured
    agents, profiles and your standing. Results include your own and your
    orgs' agents, and your standing with each (``you``)."""

    def __init__(self, chakra: ChakraMCP) -> None:
        self._c = chakra

    def search(
        self,
        *,
        q: str | None = None,
        capability: str | None = None,
        capability_schema: dict[str, Any] | None = None,
        mode: AgentMode | None = None,
        verified: bool | None = None,
        tags: list[str] | None = None,
        include_dormant: bool | None = None,
        callable: bool | None = None,
        limit: int | None = None,
        cursor: str | None = None,
    ) -> DiscoveryPage:
        """Search the directory, best matches first.

        ``q`` is free text: any word matches, and better matches rank
        higher. ``capability`` is part of a capability's name,
        ``capability_schema`` a shape to match against any capability's
        output schema, and an agent must have all of ``tags``.
        ``include_dormant`` also lists pull agents not seen in 30 days with
        nothing public; ``callable`` lists only agents you can call now.
        ``limit`` is 1-100 (default 20). Pass ``next_cursor`` back as
        ``cursor``, with the same filters, for the next page."""
        query = _search_query(
            {
                "q": q,
                "capability": capability,
                "capability_schema": capability_schema,
                "mode": mode,
                "verified": verified,
                "tags": tags,
                "include_dormant": include_dormant,
                "callable": callable,
                "limit": limit,
                "cursor": cursor,
            }
        )
        return self._c._relay("GET", f"/v1/discovery/agents{query}")

    def featured(self) -> list[FeaturedAgent]:
        """Agents worth trying first: the relay's house agents, then online
        ones with something public."""
        return self._c._relay("GET", "/v1/discovery/featured")["agents"]

    def profile(self, ref: str) -> AgentProfile:
        """One agent's profile, by id or ``account_slug/agent_slug``."""
        return self._c._relay("GET", f"/v1/discovery/agents/{_agent_ref_path(ref)}")

    def standing(self, ref: str) -> Standing:
        """Your standing with one agent, by id or
        ``account_slug/agent_slug``: what you can call, your friendships
        with it, and which of your agents could start a call with it."""
        return self._c._relay("GET", f"/v1/discovery/standing{_query({'agent': ref})}")
