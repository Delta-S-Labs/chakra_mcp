"""Claims and deadlines (relays 0.7+): ``release`` hands a claim back,
``expires_in`` shortens a call's deadline, and invocations carry ``peer``,
``expires_at``, ``lease_until`` and ``deliveries``. A respx mock stands in
for the relay."""

from __future__ import annotations

import json
from typing import Any

import pytest
import respx

from chakramcp import AsyncChakraMCP, ChakraMCP, ChakraMCPError

APP = "http://app.test"
RELAY = "http://relay.test"


def _client() -> ChakraMCP:
    return ChakraMCP(api_key="ck_test", app_url=APP, relay_url=RELAY)


def _aclient() -> AsyncChakraMCP:
    return AsyncChakraMCP(api_key="ck_test", app_url=APP, relay_url=RELAY)


def _body(route: respx.Route, i: int = -1) -> Any:
    return json.loads(route.calls[i].request.content)


PENDING = {"id": "inv-1", "status": "pending", "deliveries": 0, "lease_until": None}
REFUSED = {
    "error": {
        "code": "chk.invocation.not_releasable",
        "message": "only a claimed invocation can be released",
    }
}


@respx.mock
def test_release_posts_and_returns_the_invocation() -> None:
    route = respx.post(f"{RELAY}/v1/invocations/inv-1/release").respond(200, json=PENDING)
    with _client() as c:
        inv = c.inbox.release("inv-1")
    assert inv["status"] == "pending"
    assert inv["deliveries"] == 0
    assert _body(route) == {}


@respx.mock
def test_a_refused_release_raises_with_the_code() -> None:
    respx.post(f"{RELAY}/v1/invocations/inv-1/release").respond(409, json=REFUSED)
    with _client() as c, pytest.raises(ChakraMCPError) as raised:
        c.inbox.release("inv-1")
    assert raised.value.status == 409
    assert raised.value.code == "chk.invocation.not_releasable"


@respx.mock
async def test_async_release() -> None:
    respx.post(f"{RELAY}/v1/invocations/inv-1/release").respond(200, json=PENDING)
    async with _aclient() as c:
        inv = await c.inbox.release("inv-1")
    assert inv["status"] == "pending"


@respx.mock
def test_invoke_sends_expires_in_and_reads_peer() -> None:
    peer = {"delivery": "pull", "online": False, "last_seen_at": "2026-10-29T09:12:03Z"}
    route = respx.post(f"{RELAY}/v1/invoke").respond(
        202, json={"invocation_id": "inv-1", "status": "pending", "error": None, "peer": peer}
    )
    with _client() as c:
        res = c.invoke(
            {"grant_id": "g-1", "grantee_agent_id": "a-1", "input": {}, "expires_in": 600}
        )
    assert res["peer"] == peer
    assert _body(route)["expires_in"] == 600
