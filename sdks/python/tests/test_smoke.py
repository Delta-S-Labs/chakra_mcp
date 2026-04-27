"""Hermetic smoke — a respx mock stands in for the backend so the suite
runs in CI without a server. Validates request shape, error decoding,
and the polling / serve helpers.
"""

from __future__ import annotations

import asyncio
import json

import httpx
import pytest
import respx

from chakramcp import (
    AsyncChakraMCP,
    ChakraMCP,
    ChakraMCPError,
)


@pytest.fixture()
def app_url() -> str:
    return "http://app.test"


@pytest.fixture()
def relay_url() -> str:
    return "http://relay.test"


def test_rejects_bad_api_key() -> None:
    with pytest.raises(ValueError, match="ck_"):
        ChakraMCP(api_key="not-a-key")


@respx.mock
def test_sync_me_sets_bearer(app_url: str, relay_url: str) -> None:
    route = respx.get(f"{app_url}/v1/me").respond(
        200,
        json={
            "user": {
                "id": "u1",
                "email": "alice@example.com",
                "display_name": "Alice",
                "avatar_url": None,
                "is_admin": False,
            },
            "memberships": [],
            "survey_required": False,
        },
    )
    with ChakraMCP(api_key="ck_test", app_url=app_url, relay_url=relay_url) as c:
        me = c.me()
    assert me["user"]["email"] == "alice@example.com"
    assert route.called
    assert route.calls.last.request.headers["authorization"] == "Bearer ck_test"


@respx.mock
def test_sync_error_envelope_decoded(app_url: str, relay_url: str) -> None:
    respx.get(f"{relay_url}/v1/agents").respond(
        403,
        json={"error": {"code": "forbidden", "message": "forbidden"}},
    )
    with (
        ChakraMCP(api_key="ck_test", app_url=app_url, relay_url=relay_url) as c,
        pytest.raises(ChakraMCPError) as ei,
    ):
        c.agents.list()
    assert ei.value.status == 403
    assert ei.value.code == "forbidden"


@respx.mock
def test_sync_invoke_and_wait_polls_until_terminal(app_url: str, relay_url: str) -> None:
    respx.post(f"{relay_url}/v1/invoke").respond(
        200, json={"invocation_id": "inv1", "status": "pending", "error": None}
    )
    poll_calls = {"n": 0}

    def _poll(_request: httpx.Request) -> httpx.Response:
        poll_calls["n"] += 1
        status = "in_progress" if poll_calls["n"] < 2 else "succeeded"
        return httpx.Response(
            200,
            json={
                "id": "inv1",
                "grant_id": "g1",
                "granter_agent_id": "a1",
                "granter_display_name": "Alice Bot",
                "grantee_agent_id": "a2",
                "grantee_display_name": "Bob Bot",
                "capability_id": "c1",
                "capability_name": "echo",
                "status": status,
                "elapsed_ms": 100,
                "error_message": None,
                "input_preview": {"hello": "world"},
                "output_preview": {"echoed": "world"} if status == "succeeded" else None,
                "created_at": "2026-01-01T00:00:00Z",
                "claimed_at": None,
                "i_served": False,
                "i_invoked": True,
            },
        )

    respx.get(f"{relay_url}/v1/invocations/inv1").mock(side_effect=_poll)

    with ChakraMCP(api_key="ck_test", app_url=app_url, relay_url=relay_url) as c:
        final = c.invoke_and_wait(
            {"grant_id": "g1", "grantee_agent_id": "a2", "input": {"hello": "world"}},
            interval_s=0.01,
            timeout_s=5.0,
        )
    assert final["status"] == "succeeded"
    assert final["output_preview"] == {"echoed": "world"}


@respx.mock
def test_sync_inbox_serve_handles_one_then_stops(app_url: str, relay_url: str) -> None:
    pull_count = {"n": 0}

    def _pull(_req: httpx.Request) -> httpx.Response:
        pull_count["n"] += 1
        if pull_count["n"] > 1:
            return httpx.Response(200, json=[])
        return httpx.Response(
            200,
            json=[
                {
                    "id": "inv1",
                    "grant_id": None,
                    "granter_agent_id": None,
                    "granter_display_name": None,
                    "grantee_agent_id": None,
                    "grantee_display_name": None,
                    "capability_id": None,
                    "capability_name": "echo",
                    "status": "in_progress",
                    "elapsed_ms": 0,
                    "error_message": None,
                    "input_preview": {"hello": "world"},
                    "output_preview": None,
                    "created_at": "2026-01-01T00:00:00Z",
                    "claimed_at": "2026-01-01T00:00:01Z",
                    "i_served": True,
                    "i_invoked": False,
                }
            ],
        )

    reported: list[tuple[str, str]] = []

    def _result(req: httpx.Request) -> httpx.Response:
        body = json.loads(req.content)
        inv_id = req.url.path.split("/")[3]
        reported.append((inv_id, body["status"]))
        return httpx.Response(200, json={})

    respx.get(f"{relay_url}/v1/inbox").mock(side_effect=_pull)
    respx.post(f"{relay_url}/v1/invocations/inv1/result").mock(side_effect=_result)

    handler_seen: list[str] = []
    stopped = {"flag": False}

    def handler(inv: dict) -> dict:
        handler_seen.append(inv["id"])
        # Stop after the first invocation is dispatched.
        stopped["flag"] = True
        return {"status": "succeeded", "output": {"ok": True}}

    with ChakraMCP(api_key="ck_test", app_url=app_url, relay_url=relay_url) as c:
        c.inbox.serve(
            "agent-id",
            handler,
            poll_interval_s=0.01,
            stop=lambda: stopped["flag"] and pull_count["n"] >= 1,
        )
    assert handler_seen == ["inv1"]
    assert reported == [("inv1", "succeeded")]


@respx.mock
async def test_async_me(app_url: str, relay_url: str) -> None:
    respx.get(f"{app_url}/v1/me").respond(
        200,
        json={
            "user": {
                "id": "u2",
                "email": "bob@example.com",
                "display_name": "Bob",
                "avatar_url": None,
                "is_admin": False,
            },
            "memberships": [],
            "survey_required": False,
        },
    )
    async with AsyncChakraMCP(api_key="ck_test", app_url=app_url, relay_url=relay_url) as c:
        me = await c.me()
    assert me["user"]["email"] == "bob@example.com"


@respx.mock
async def test_async_inbox_serve_loops_then_stops(app_url: str, relay_url: str) -> None:
    pull_count = {"n": 0}

    def _pull(_req: httpx.Request) -> httpx.Response:
        pull_count["n"] += 1
        if pull_count["n"] > 1:
            return httpx.Response(200, json=[])
        return httpx.Response(
            200,
            json=[
                {
                    "id": "inv2",
                    "grant_id": None,
                    "granter_agent_id": None,
                    "granter_display_name": None,
                    "grantee_agent_id": None,
                    "grantee_display_name": None,
                    "capability_id": None,
                    "capability_name": "echo",
                    "status": "in_progress",
                    "elapsed_ms": 0,
                    "error_message": None,
                    "input_preview": {"hi": "there"},
                    "output_preview": None,
                    "created_at": "2026-01-01T00:00:00Z",
                    "claimed_at": "2026-01-01T00:00:01Z",
                    "i_served": True,
                    "i_invoked": False,
                }
            ],
        )

    respx.get(f"{relay_url}/v1/inbox").mock(side_effect=_pull)
    respx.post(f"{relay_url}/v1/invocations/inv2/result").respond(200, json={})

    stop_event = asyncio.Event()
    handler_seen: list[str] = []

    async def handler(inv: dict) -> dict:
        handler_seen.append(inv["id"])
        stop_event.set()
        return {"status": "succeeded", "output": {"ok": True}}

    async with AsyncChakraMCP(api_key="ck_test", app_url=app_url, relay_url=relay_url) as c:
        await c.inbox.serve(
            "agent-id",
            handler,
            poll_interval_s=0.01,
            stop_event=stop_event,
        )
    assert handler_seen == ["inv2"]
