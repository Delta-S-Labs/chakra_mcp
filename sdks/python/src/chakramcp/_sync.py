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
    EligibilityResponse,
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
    Review,
    ReviewListResponse,
    ReviewTier,
    UpdateAgentRequest,
    WriteReviewRequest,
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
        self.reviews = ReviewsClient(self)

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

    def respond(self, invocation_id: str, result: HandlerResult | dict[str, Any]) -> Invocation:
        return self._c._relay(
            "POST", f"/v1/invocations/{invocation_id}/result", dict(result)
        )

    def serve(
        self,
        agent_id: str,
        handler: Callable[[Invocation], HandlerResult | dict[str, Any]],
        *,
        human_handler: Callable[[Invocation], None] | None = None,
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

        ``human_handler`` is the human-in-the-loop path. When an
        invocation arrives with ``semantics == "human_in_loop"``, the
        SDK invokes ``human_handler`` (e.g. to enqueue a notification,
        write a pending-task file, page someone) and then **moves on
        without posting a result** — the invocation row stays
        ``in_progress`` until a human resolves it out-of-band via
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
                # HITL routing — defer to human_handler and skip
                # respond(). The row stays in_progress; a human
                # resolves it later via the CLI reply path which
                # sets `confirmed_by_human: true` on the result.
                if inv.get("semantics") == "human_in_loop":
                    if human_handler is not None:
                        try:
                            human_handler(inv)
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
