"""Waiting pulls: ``wait`` and ``lease_secs`` on the pull feeds, the
``ChakraMCP-Wait`` header, and ``serve`` pulling again at once while the
relay waits. A respx mock stands in for the relay."""

from __future__ import annotations

import asyncio
import time
from collections.abc import Callable
from typing import Any

import httpx
import pytest
import respx

from chakramcp import (
    AsyncChakraMCP,
    ChakraMCP,
    ChakraMCPError,
    Invocation,
    PullResult,
)

APP = "http://app.test"
RELAY = "http://relay.test"
AGENT = "agent-1"
WAITED = {"ChakraMCP-Wait": "25"}


def _client(**kwargs: Any) -> ChakraMCP:
    return ChakraMCP(api_key="ck_test", app_url=APP, relay_url=RELAY, **kwargs)


def _aclient(**kwargs: Any) -> AsyncChakraMCP:
    return AsyncChakraMCP(api_key="ck_test", app_url=APP, relay_url=RELAY, **kwargs)


def _params(route: respx.Route, i: int = -1) -> dict[str, str]:
    return dict(route.calls[i].request.url.params)


def _record_sleeps(monkeypatch: pytest.MonkeyPatch) -> list[float]:
    """Make ``time.sleep`` return at once, and record what it was asked."""
    sleeps: list[float] = []
    monkeypatch.setattr(time, "sleep", sleeps.append)
    return sleeps


def _handler(inv: Invocation) -> dict[str, Any]:
    raise AssertionError(f"no invocation expected, got {inv['id']}")


async def _ahandler(inv: Invocation) -> dict[str, Any]:
    raise AssertionError(f"no invocation expected, got {inv['id']}")


async def _until(pred: Callable[[], bool]) -> None:
    async def poll() -> None:
        while not pred():
            await asyncio.sleep(0.005)

    await asyncio.wait_for(poll(), 5)


_INV = {"id": "inv-1", "capability_name": "echo", "status": "in_progress"}


# ─── pull, pull_with_meta and notices ─────────────────────


@respx.mock
def test_pull_sends_wait_and_lease_secs_only_when_set() -> None:
    route = respx.get(f"{RELAY}/v1/inbox").respond(200, json=[_INV], headers=WAITED)
    with _client() as c:
        assert c.inbox.pull(AGENT, limit=5, wait=25, lease_secs=1800) == [_INV]
        assert _params(route) == {
            "agent_id": AGENT,
            "limit": "5",
            "wait": "25",
            "lease_secs": "1800",
        }
        c.inbox.pull(AGENT)
        assert _params(route) == {"agent_id": AGENT}
        c.inbox.pull(AGENT, wait=0)
        assert _params(route) == {"agent_id": AGENT, "wait": "0"}


@respx.mock
async def test_async_pull_sends_wait_and_lease_secs_only_when_set() -> None:
    route = respx.get(f"{RELAY}/v1/inbox").respond(200, json=[_INV], headers=WAITED)
    async with _aclient() as c:
        assert await c.inbox.pull(AGENT, limit=5, wait=10, lease_secs=600) == [_INV]
        assert _params(route) == {
            "agent_id": AGENT,
            "limit": "5",
            "wait": "10",
            "lease_secs": "600",
        }
        await c.inbox.pull(AGENT)
        assert _params(route) == {"agent_id": AGENT}


# Header sent by the relay -> what ``waited`` reads.
_HEADERS: list[tuple[dict[str, str], int | None]] = [
    ({"ChakraMCP-Wait": "25"}, 25),
    ({"chakramcp-wait": "7"}, 7),
    ({"CHAKRAMCP-WAIT": "0"}, 0),
    ({}, None),
    ({"ChakraMCP-Wait": "soon"}, None),
    ({"ChakraMCP-Wait": "2.5"}, None),
]


@respx.mock
def test_pull_with_meta_reads_the_wait_header() -> None:
    respx.get(f"{RELAY}/v1/inbox").mock(
        side_effect=[httpx.Response(200, json=[_INV], headers=h) for h, _ in _HEADERS]
    )
    with _client() as c:
        for _, expected in _HEADERS:
            pulled: PullResult[Invocation] = c.inbox.pull_with_meta(AGENT, wait=25)
            assert pulled == {"items": [_INV], "waited": expected}


@respx.mock
async def test_async_pull_with_meta_reads_the_wait_header() -> None:
    respx.get(f"{RELAY}/v1/inbox").mock(
        side_effect=[httpx.Response(200, json=[], headers=h) for h, _ in _HEADERS]
    )
    async with _aclient() as c:
        for _, expected in _HEADERS:
            assert await c.inbox.pull_with_meta(AGENT, wait=25) == {
                "items": [],
                "waited": expected,
            }


@respx.mock
def test_pull_with_meta_still_raises_relay_errors() -> None:
    respx.get(f"{RELAY}/v1/inbox").respond(
        403, json={"error": {"code": "forbidden", "message": "not your agent"}}
    )
    with _client() as c, pytest.raises(ChakraMCPError) as ei:
        c.inbox.pull_with_meta(AGENT, wait=25)
    assert (ei.value.status, ei.value.code) == (403, "forbidden")


@respx.mock
def test_notices_send_wait_and_carry_invocation_id() -> None:
    notice = {
        "id": "n1",
        "kind": "invocation_expired",
        "created_at": "2026-10-29T00:00:00Z",
        "invocation_id": "inv-1",
    }
    route = respx.get(f"{RELAY}/v1/inbox/notices").respond(200, json=[notice], headers=WAITED)
    with _client() as c:
        notices = c.inbox.notices(AGENT, limit=5, wait=20)
    assert notices[0]["invocation_id"] == "inv-1"
    assert _params(route) == {"agent_id": AGENT, "limit": "5", "wait": "20"}


@respx.mock
async def test_async_notices_send_wait() -> None:
    route = respx.get(f"{RELAY}/v1/inbox/notices").respond(200, json=[], headers=WAITED)
    async with _aclient() as c:
        assert await c.inbox.notices(AGENT, wait=20) == []
        assert _params(route) == {"agent_id": AGENT, "wait": "20"}
        await c.inbox.notices(AGENT)
        assert _params(route) == {"agent_id": AGENT}


@respx.mock
def test_calls_pull_sends_wait_and_pull_with_meta_reads_the_header() -> None:
    route = respx.get(f"{RELAY}/v1/inbox/calls").mock(
        side_effect=[httpx.Response(200, json=[], headers=WAITED), httpx.Response(200, json=[])]
    )
    with _client() as c:
        assert c.calls.pull_with_meta(AGENT, limit=10, wait=25) == {"items": [], "waited": 25}
        assert _params(route) == {"agent_id": AGENT, "limit": "10", "wait": "25"}
        assert c.calls.pull(AGENT) == []
        assert _params(route) == {"agent_id": AGENT}


@respx.mock
async def test_async_calls_pull_sends_wait_and_pull_with_meta_reads_the_header() -> None:
    route = respx.get(f"{RELAY}/v1/inbox/calls").mock(
        side_effect=[httpx.Response(200, json=[], headers=WAITED), httpx.Response(200, json=[])]
    )
    async with _aclient() as c:
        assert await c.calls.pull_with_meta(AGENT, wait=25) == {"items": [], "waited": 25}
        assert _params(route) == {"agent_id": AGENT, "wait": "25"}
        assert await c.calls.pull_with_meta(AGENT) == {"items": [], "waited": None}


@respx.mock
def test_a_waiting_pull_outlasts_a_short_read_timeout() -> None:
    route = respx.get(f"{RELAY}/v1/inbox").respond(200, json=[])

    def read_timeout() -> float:
        return float(route.calls.last.request.extensions["timeout"]["read"])

    # httpx's default read timeout is 5 seconds: a 25-second wait gets 35.
    with httpx.Client() as http, _client(http_client=http) as c:
        c.inbox.pull(AGENT, wait=25)
        assert read_timeout() == 35
        c.inbox.pull(AGENT)
        assert read_timeout() == 5
    # The SDK's own client reads for 60 seconds already.
    with _client() as c:
        c.inbox.pull(AGENT, wait=25)
        assert read_timeout() == 60


def test_pull_result_can_be_subscripted_at_run_time() -> None:
    # Python 3.10 can't make a TypedDict generic; the SDK keeps this working.
    assert PullResult[Invocation] is not None


# ─── inbox.serve ──────────────────────────────────────────


@respx.mock
def test_inbox_serve_pulls_again_at_once_while_the_relay_waits(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    sleeps = _record_sleeps(monkeypatch)
    route = respx.get(f"{RELAY}/v1/inbox").respond(200, json=[], headers=WAITED)
    with _client() as c:
        c.inbox.serve(
            AGENT,
            _handler,
            notice_interval_s=0,
            poll_interval_s=30,
            stop=lambda: route.call_count >= 3,
        )
    assert route.call_count == 3
    assert sleeps == []


@pytest.mark.parametrize("headers", [{}, {"ChakraMCP-Wait": "0"}], ids=["old-relay", "no-wait"])
@respx.mock
def test_inbox_serve_sleeps_when_the_relay_did_not_wait(
    monkeypatch: pytest.MonkeyPatch, headers: dict[str, str]
) -> None:
    sleeps = _record_sleeps(monkeypatch)
    route = respx.get(f"{RELAY}/v1/inbox").respond(200, json=[], headers=headers)
    with _client() as c:
        c.inbox.serve(
            AGENT,
            _handler,
            notice_interval_s=0,
            poll_interval_s=30,
            stop=lambda: route.call_count >= 3,
        )
    assert sleeps == [30, 30, 30]


@pytest.mark.parametrize(
    ("serve_kwargs", "expected"),
    [
        ({"notice_interval_s": 0}, {"wait": "25"}),
        ({"notice_interval_s": 0, "lease_secs": 1800}, {"wait": "25", "lease_secs": "1800"}),
        # Capped to when notices are next due, so they still come on time.
        ({}, {"wait": "15"}),
        ({"notice_interval_s": 5, "wait_s": 3}, {"wait": "3"}),
        ({"notice_interval_s": 0.5}, {"wait": "1"}),
        # 0 never waits: no ``wait`` at all.
        ({"notice_interval_s": 0, "wait_s": 0}, {}),
    ],
    ids=["default", "lease", "notices", "wait_s", "short-notices", "off"],
)
@respx.mock
def test_inbox_serve_asks_the_relay_to_wait(
    serve_kwargs: dict[str, Any], expected: dict[str, str]
) -> None:
    respx.get(f"{RELAY}/v1/inbox/notices").respond(200, json=[])
    route = respx.get(f"{RELAY}/v1/inbox").respond(200, json=[], headers=WAITED)
    with _client() as c:
        c.inbox.serve(AGENT, _handler, stop=lambda: route.call_count >= 1, **serve_kwargs)
    assert _params(route, 0) == {"agent_id": AGENT, "limit": "25", **expected}


@respx.mock
async def test_async_inbox_serve_pulls_again_at_once_while_the_relay_waits() -> None:
    stop = asyncio.Event()
    pulls = {"n": 0}

    def _pull(_req: httpx.Request) -> httpx.Response:
        pulls["n"] += 1
        if pulls["n"] == 3:
            stop.set()
        return httpx.Response(200, json=[], headers=WAITED)

    route = respx.get(f"{RELAY}/v1/inbox").mock(side_effect=_pull)
    async with _aclient() as c:
        # A sleep of poll_interval_s would trip the 5-second limit.
        await asyncio.wait_for(
            c.inbox.serve(
                AGENT, _ahandler, notice_interval_s=0, poll_interval_s=30, stop_event=stop
            ),
            5,
        )
    assert route.call_count == 3
    assert [_params(route, i)["wait"] for i in range(3)] == ["25", "25", "25"]


@respx.mock
async def test_async_inbox_serve_sleeps_when_the_relay_did_not_wait() -> None:
    route = respx.get(f"{RELAY}/v1/inbox").respond(200, json=[])
    async with _aclient() as c:
        task = asyncio.create_task(
            c.inbox.serve(AGENT, _ahandler, notice_interval_s=0, poll_interval_s=30)
        )
        await _until(lambda: route.call_count > 0)
        await asyncio.sleep(0.1)
        # Asleep for poll_interval_s after the empty pull.
        assert route.call_count == 1
        task.cancel()
        await asyncio.wait({task})
        assert task.cancelled()


@respx.mock
async def test_async_inbox_serve_caps_the_wait_to_the_notice_interval() -> None:
    respx.get(f"{RELAY}/v1/inbox/notices").respond(200, json=[])
    stop = asyncio.Event()

    def _pull(_req: httpx.Request) -> httpx.Response:
        stop.set()
        return httpx.Response(200, json=[], headers=WAITED)

    route = respx.get(f"{RELAY}/v1/inbox").mock(side_effect=_pull)
    async with _aclient() as c:
        await asyncio.wait_for(
            c.inbox.serve(AGENT, _ahandler, lease_secs=900, stop_event=stop), 5
        )
    assert _params(route, 0) == {
        "agent_id": AGENT,
        "limit": "25",
        "wait": "15",
        "lease_secs": "900",
    }


# ─── calls.serve ──────────────────────────────────────────


@respx.mock
def test_calls_serve_pulls_again_at_once_while_the_relay_waits(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    sleeps = _record_sleeps(monkeypatch)
    route = respx.get(f"{RELAY}/v1/inbox/calls").respond(200, json=[], headers=WAITED)
    with _client() as c:
        c.calls.serve(AGENT, poll_interval_s=30, stop=lambda: route.call_count >= 3)
    assert route.call_count == 3
    assert sleeps == []
    assert [_params(route, i) for i in range(3)] == [
        {"agent_id": AGENT, "limit": "25", "wait": "25"}
    ] * 3


@pytest.mark.parametrize("headers", [{}, {"ChakraMCP-Wait": "0"}], ids=["old-relay", "no-wait"])
@respx.mock
def test_calls_serve_sleeps_when_the_relay_did_not_wait(
    monkeypatch: pytest.MonkeyPatch, headers: dict[str, str]
) -> None:
    sleeps = _record_sleeps(monkeypatch)
    route = respx.get(f"{RELAY}/v1/inbox/calls").respond(200, json=[], headers=headers)
    with _client() as c:
        c.calls.serve(AGENT, poll_interval_s=30, stop=lambda: route.call_count >= 3)
    assert sleeps == [30, 30, 30]


@respx.mock
def test_calls_serve_with_wait_s_0_does_not_ask_to_wait(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _record_sleeps(monkeypatch)
    route = respx.get(f"{RELAY}/v1/inbox/calls").respond(200, json=[])
    with _client() as c:
        c.calls.serve(AGENT, wait_s=0, stop=lambda: route.call_count >= 1)
    assert _params(route, 0) == {"agent_id": AGENT, "limit": "25"}


@respx.mock
async def test_async_calls_serve_pulls_again_at_once_while_the_relay_waits() -> None:
    stop = asyncio.Event()
    pulls = {"n": 0}

    def _pull(_req: httpx.Request) -> httpx.Response:
        pulls["n"] += 1
        if pulls["n"] == 3:
            stop.set()
        return httpx.Response(200, json=[], headers=WAITED)

    route = respx.get(f"{RELAY}/v1/inbox/calls").mock(side_effect=_pull)
    async with _aclient() as c:
        await asyncio.wait_for(c.calls.serve(AGENT, poll_interval_s=30, stop_event=stop), 5)
    assert route.call_count == 3
    assert [_params(route, i)["wait"] for i in range(3)] == ["25", "25", "25"]


@respx.mock
async def test_async_calls_serve_sleeps_when_the_relay_did_not_wait() -> None:
    route = respx.get(f"{RELAY}/v1/inbox/calls").respond(200, json=[])
    async with _aclient() as c:
        task = asyncio.create_task(c.calls.serve(AGENT, poll_interval_s=30))
        await _until(lambda: route.call_count > 0)
        await asyncio.sleep(0.1)
        assert route.call_count == 1
        task.cancel()
        await asyncio.wait({task})
        assert task.cancelled()
