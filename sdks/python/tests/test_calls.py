"""Calls: the ``calls`` namespace on both clients, ``calls.serve``, and
``call_id`` on invoke. A respx mock stands in for the relay."""

from __future__ import annotations

import asyncio
import json
import logging
import threading
import time
from typing import Any

import httpx
import pytest
import respx

from chakramcp import AsyncChakraMCP, ChakraMCP, ChakraMCPError, InviteDecision

APP = "http://app.test"
RELAY = "http://relay.test"
ME = "a-callee"


def _client() -> ChakraMCP:
    return ChakraMCP(api_key="ck_test", app_url=APP, relay_url=RELAY)


def _aclient() -> AsyncChakraMCP:
    return AsyncChakraMCP(api_key="ck_test", app_url=APP, relay_url=RELAY)


def _call(call_id: str = "call-1", state: str = "ringing") -> dict[str, Any]:
    return {
        "id": call_id,
        "state": state,
        "caller": {
            "agent_id": "a-caller",
            "account_id": "acc-1",
            "display_name": "Scheduler",
            "account_slug": "alice",
        },
        "callee": {
            "agent_id": ME,
            "account_id": "acc-2",
            "display_name": "Planner",
            "account_slug": "bob",
        },
        "memory": "call",
        "memory_features": {"tags": True, "recall": True},
        "ttl_secs": 2592000,
        "idle_secs": 86400,
        "topic": "Offsite venue",
        "created_at": "2026-10-26T10:00:00Z",
        "answered_at": None,
        "ended_at": None,
        "ended_by": None,
        "purge_at": None,
        "purged_at": None,
        "message_count": 0,
        "last_message_at": None,
    }


def _entry(
    seq: int,
    kind: str,
    text: str | None = None,
    *,
    from_agent_id: str | None = "a-caller",
    data: Any = None,
) -> dict[str, Any]:
    return {
        "seq": seq,
        "kind": kind,
        "from_agent_id": from_agent_id,
        "text": text,
        "data": data,
        "blocked": False,
        "created_at": "2026-10-26T10:00:00Z",
    }


def _item(call_id: str, seq: int, kind: str, text: str | None = None) -> dict[str, Any]:
    state = {"invite": "ringing", "ended": "ended"}.get(kind, "active")
    return {"call": _call(call_id, state), "entry": _entry(seq, kind, text)}


def _body(route: respx.Route) -> Any:
    return json.loads(route.calls.last.request.content)


def _feed(*batches: list[dict[str, Any]]) -> tuple[dict[str, int], Any]:
    """A calls feed that serves ``batches`` in turn, then empty ones."""
    pulls = {"n": 0}

    def _pull(_req: httpx.Request) -> httpx.Response:
        pulls["n"] += 1
        i = pulls["n"] - 1
        return httpx.Response(200, json=batches[i] if i < len(batches) else [])

    return pulls, _pull


# ─── The namespace ────────────────────────────────────────


@respx.mock
def test_start_sends_the_body_and_parses_a_call() -> None:
    route = respx.post(f"{RELAY}/v1/calls").respond(201, json=_call())
    body = {
        "from_agent_id": "a-caller",
        "to_agent_id": ME,
        "memory": "call",
        "ttl_secs": 2592000,
        "topic": "Offsite venue",
        "message": "Hall A or B?",
    }
    with _client() as c:
        call = c.calls.start(body)
    assert _body(route) == body
    assert call["id"] == "call-1"
    assert call["state"] == "ringing"
    assert call["memory_features"] == {"tags": True, "recall": True}
    assert call["callee"]["display_name"] == "Planner"


@respx.mock
def test_list_passes_the_query_and_reads_next_cursor() -> None:
    route = respx.get(f"{RELAY}/v1/calls").respond(
        200, json={"calls": [_call(state="active")], "next_cursor": "c2Vjb25k+/="}
    )
    with _client() as c:
        page = c.calls.list(agent_id=ME, state="active", limit=10, cursor="Zmlyc3Q+/=")
        assert dict(route.calls.last.request.url.params) == {
            "agent_id": ME,
            "state": "active",
            "limit": "10",
            "cursor": "Zmlyc3Q+/=",
        }
        assert page["calls"][0]["state"] == "active"
        assert page["next_cursor"] == "c2Vjb25k+/="

        c.calls.list()
        assert route.calls.last.request.url.query == b""


@respx.mock
def test_send_and_history_round_trip() -> None:
    sent = respx.post(f"{RELAY}/v1/calls/call-1/messages").respond(
        201,
        json=_entry(3, "message", "Hall A works", from_agent_id=ME, data={"hall": "A"}),
    )
    history = respx.get(f"{RELAY}/v1/calls/call-1/messages").respond(
        200,
        json=[
            _entry(1, "invite", "Hall A or B?"),
            _entry(2, "accepted", from_agent_id=ME),
            _entry(3, "message", "Hall A works", from_agent_id=ME, data={"hall": "A"}),
        ],
    )
    with _client() as c:
        entry = c.calls.send("call-1", ME, "Hall A works", data={"hall": "A"})
        assert _body(sent) == {"agent_id": ME, "text": "Hall A works", "data": {"hall": "A"}}
        assert entry["seq"] == 3
        assert entry["kind"] == "message"
        assert entry["data"] == {"hall": "A"}

        c.calls.send("call-1", ME, "No data")
        assert _body(sent) == {"agent_id": ME, "text": "No data"}

        entries = c.calls.history("call-1", after=0, limit=50, q="hall A")
    request = history.calls.last.request
    assert dict(request.url.params) == {"after": "0", "limit": "50", "q": "hall A"}
    assert b"q=hall+A" in request.url.query
    assert [e["seq"] for e in entries] == [1, 2, 3]
    assert entries[2]["text"] == "Hall A works"


@respx.mock
def test_ask_parses_a_recall() -> None:
    route = respx.post(f"{RELAY}/v1/calls/call-1/memory/ask").respond(
        200,
        json={
            "answered": 0.93,
            "messages": [
                {
                    "seq": 7,
                    "from": ME,
                    "text": "Hall A, agreed.",
                    "at": "2026-10-26T10:05:00Z",
                    "p": 0.88,
                }
            ],
        },
    )
    with _client() as c:
        recall = c.calls.ask("call-1", "a-caller", "Which hall did we pick?")
    assert _body(route) == {"agent_id": "a-caller", "question": "Which hall did we pick?"}
    assert recall["answered"] == 0.93
    assert recall["messages"][0]["seq"] == 7
    assert recall["messages"][0]["from"] == ME
    assert recall["messages"][0]["p"] == 0.88


@pytest.mark.parametrize(
    ("status", "code"),
    [
        (409, "chk.calls.not_active"),
        (403, "chk.calls.not_accepted"),
        (503, "chk.calls.memory_unavailable"),
    ],
)
@respx.mock
def test_a_calls_error_code_surfaces(status: int, code: str) -> None:
    respx.post(f"{RELAY}/v1/calls/call-1/accept").respond(
        status, json={"error": {"code": code, "message": "refused"}}
    )
    with _client() as c, pytest.raises(ChakraMCPError) as ei:
        c.calls.accept("call-1", ME)
    assert ei.value.status == status
    assert ei.value.code == code
    assert ei.value.message == "refused"


@respx.mock
def test_the_other_methods_hit_their_routes() -> None:
    got = respx.get(f"{RELAY}/v1/calls/call-1").respond(200, json=_call())
    accept = respx.post(f"{RELAY}/v1/calls/call-1/accept").respond(
        200, json=_call(state="active")
    )
    decline = respx.post(f"{RELAY}/v1/calls/call-2/decline").respond(
        200, json=_call("call-2", "declined")
    )
    end = respx.post(f"{RELAY}/v1/calls/call-1/end").respond(200, json=_call(state="ended"))
    memory = respx.get(f"{RELAY}/v1/calls/call-1/memory").respond(
        200,
        json={
            "pinned": [
                {
                    "seq": 7,
                    "from": ME,
                    "at": "2026-10-26T10:05:00Z",
                    "text": "Agreed.",
                    "context": "Hall A?",
                    "tags": {
                        "decision": 0.9,
                        "commitment": 0.2,
                        "open_question": 0.0,
                        "fact": 0.1,
                        "weight": 0.8,
                    },
                }
            ],
            "open": [],
            "recent": [],
        },
    )
    pull = respx.get(f"{RELAY}/v1/inbox/calls").respond(
        200, json=[_item("call-1", 1, "invite", "Hall A or B?")]
    )
    with _client() as c:
        assert c.calls.get("call-1")["id"] == "call-1"
        assert got.called

        assert c.calls.accept("call-1", ME, message="Hello")["state"] == "active"
        assert _body(accept) == {"agent_id": ME, "message": "Hello"}
        c.calls.accept("call-1", ME)
        assert _body(accept) == {"agent_id": ME}

        assert c.calls.decline("call-2", ME, reason="Busy")["state"] == "declined"
        assert _body(decline) == {"agent_id": ME, "reason": "Busy"}

        assert c.calls.end("call-1", ME)["state"] == "ended"
        assert _body(end) == {"agent_id": ME}

        bundle = c.calls.memory("call-1")
        assert memory.called
        assert bundle["pinned"][0]["context"] == "Hall A?"
        assert bundle["pinned"][0]["tags"]["decision"] == 0.9

        items = c.calls.pull(ME, limit=5)
        assert dict(pull.calls.last.request.url.params) == {"agent_id": ME, "limit": "5"}
        assert items[0]["entry"]["kind"] == "invite"
        assert items[0]["call"]["topic"] == "Offsite venue"


@respx.mock
async def test_async_namespace_hits_the_same_routes() -> None:
    start = respx.post(f"{RELAY}/v1/calls").respond(201, json=_call())
    listed = respx.get(f"{RELAY}/v1/calls").respond(200, json={"calls": []})
    respx.get(f"{RELAY}/v1/calls/call-1").respond(200, json=_call())
    accept = respx.post(f"{RELAY}/v1/calls/call-1/accept").respond(
        200, json=_call(state="active")
    )
    decline = respx.post(f"{RELAY}/v1/calls/call-1/decline").respond(
        200, json=_call(state="declined")
    )
    end = respx.post(f"{RELAY}/v1/calls/call-1/end").respond(200, json=_call(state="ended"))
    send = respx.post(f"{RELAY}/v1/calls/call-1/messages").respond(
        201, json=_entry(4, "message", "Hi", from_agent_id=ME)
    )
    history = respx.get(f"{RELAY}/v1/calls/call-1/messages").respond(200, json=[])
    respx.get(f"{RELAY}/v1/calls/call-1/memory").respond(
        200, json={"pinned": [], "open": [], "recent": []}
    )
    ask = respx.post(f"{RELAY}/v1/calls/call-1/memory/ask").respond(
        200, json={"answered": None, "messages": []}
    )
    pull = respx.get(f"{RELAY}/v1/inbox/calls").respond(200, json=[])

    async with _aclient() as c:
        call = await c.calls.start({"from_agent_id": "a-caller", "to_agent_id": ME})
        assert call["state"] == "ringing"
        assert _body(start) == {"from_agent_id": "a-caller", "to_agent_id": ME}

        page = await c.calls.list(agent_id=ME, state="ringing")
        assert page["calls"] == []
        assert "next_cursor" not in page
        assert dict(listed.calls.last.request.url.params) == {"agent_id": ME, "state": "ringing"}

        assert (await c.calls.get("call-1"))["id"] == "call-1"
        await c.calls.accept("call-1", ME, message="Hi")
        assert _body(accept) == {"agent_id": ME, "message": "Hi"}
        await c.calls.decline("call-1", ME, reason="Busy")
        assert _body(decline) == {"agent_id": ME, "reason": "Busy"}
        await c.calls.end("call-1", ME)
        assert _body(end) == {"agent_id": ME}

        entry = await c.calls.send("call-1", ME, "Hi", data=[1, 2])
        assert entry["seq"] == 4
        assert _body(send) == {"agent_id": ME, "text": "Hi", "data": [1, 2]}
        assert await c.calls.history("call-1", q="venue") == []
        assert dict(history.calls.last.request.url.params) == {"q": "venue"}

        assert (await c.calls.memory("call-1"))["recent"] == []
        recall = await c.calls.ask("call-1", ME, "Which hall?")
        assert recall["answered"] is None
        assert _body(ask) == {"agent_id": ME, "question": "Which hall?"}

        assert await c.calls.pull(ME) == []
        assert dict(pull.calls.last.request.url.params) == {"agent_id": ME}


# ─── serve ────────────────────────────────────────────────


@respx.mock
def test_serve_accepts_replies_reports_events_and_survives_a_handler_error() -> None:
    pulls, feed = _feed(
        [
            _item("call-1", 1, "invite", "Hall A or B?"),
            _item("call-2", 5, "message", "boom"),
            _item("call-2", 6, "message", "When can you meet?"),
            # Not delivered by the relay, and not an event: skipped.
            _item("call-2", 8, "invocation"),
            _item("call-3", 9, "ended"),
        ]
    )
    pull = respx.get(f"{RELAY}/v1/inbox/calls").mock(side_effect=feed)
    accept = respx.post(f"{RELAY}/v1/calls/call-1/accept").respond(
        200, json=_call(state="active")
    )
    send = respx.post(f"{RELAY}/v1/calls/call-2/messages").respond(
        201, json=_entry(7, "message", "Tuesday", from_agent_id=ME)
    )

    seen: list[tuple[str, int]] = []
    events: list[str] = []
    errors: list[tuple[str, str | None]] = []
    stop = threading.Event()

    def on_message(ctx: Any) -> str:
        seen.append((ctx["call"]["id"], ctx["entry"]["seq"]))
        if ctx["entry"]["text"] == "boom":
            raise RuntimeError("model fell over")
        return "Tuesday"

    def on_event(ctx: Any) -> None:
        events.append(ctx["entry"]["kind"])
        stop.set()

    with _client() as c:
        c.calls.serve(
            ME,
            on_message,
            on_event=on_event,
            on_error=lambda err, ctx: errors.append((str(err), ctx and ctx["call"]["id"])),
            poll_interval_s=0.01,
            stop=stop,
        )

    # Invites are accepted by default.
    assert accept.call_count == 1
    assert _body(accept) == {"agent_id": ME}
    # The failing message is reported; the next one still gets its reply.
    assert seen == [("call-2", 5), ("call-2", 6)]
    assert errors == [("model fell over", "call-2")]
    assert send.call_count == 1
    assert _body(send) == {"agent_id": ME, "text": "Tuesday"}
    assert events == ["ended"]
    assert pull.calls.last.request.url.params["limit"] == "25"
    assert pulls["n"] == 1


@respx.mock
def test_serve_uses_on_invite_and_ignores_a_reply_to_an_ended_call(
    caplog: pytest.LogCaptureFixture,
) -> None:
    _, feed = _feed(
        [
            _item("call-1", 1, "invite"),
            _item("call-2", 1, "invite"),
            _item("call-3", 4, "message", "Still there?"),
        ]
    )
    respx.get(f"{RELAY}/v1/inbox/calls").mock(side_effect=feed)
    accept = respx.post(f"{RELAY}/v1/calls/call-1/accept").respond(
        200, json=_call(state="active")
    )
    decline = respx.post(f"{RELAY}/v1/calls/call-2/decline").respond(
        200, json=_call("call-2", "declined")
    )
    send = respx.post(f"{RELAY}/v1/calls/call-3/messages").respond(
        409, json={"error": {"code": "chk.calls.not_active", "message": "the call ended"}}
    )

    def on_invite(ctx: Any) -> InviteDecision:
        if ctx["call"]["id"] == "call-1":
            return {"accept": True, "message": "Happy to help"}
        return {"accept": False, "reason": "Not taking calls today"}

    replied: list[int] = []

    def on_message(ctx: Any) -> dict[str, Any]:
        replied.append(ctx["entry"]["seq"])
        return {"text": "Yes", "data": {"ok": True}}

    with caplog.at_level(logging.ERROR, logger="chakramcp"), _client() as c:
        c.calls.serve(
            ME,
            on_message,
            on_invite=on_invite,
            poll_interval_s=0.01,
            stop=lambda: bool(replied),
        )

    assert _body(accept) == {"agent_id": ME, "message": "Happy to help"}
    assert _body(decline) == {"agent_id": ME, "reason": "Not taking calls today"}
    assert _body(send) == {"agent_id": ME, "text": "Yes", "data": {"ok": True}}
    # The 409 was ignored: nothing logged.
    assert not [r for r in caplog.records if r.name == "chakramcp"]


@respx.mock
def test_serve_logs_a_handler_error_without_on_error(caplog: pytest.LogCaptureFixture) -> None:
    _, feed = _feed([_item("call-9", 2, "missed")])
    respx.get(f"{RELAY}/v1/inbox/calls").mock(side_effect=feed)
    calls = {"n": 0}

    def on_event(ctx: Any) -> None:
        calls["n"] += 1
        raise ValueError("bad event")

    with caplog.at_level(logging.ERROR, logger="chakramcp"), _client() as c:
        c.calls.serve(ME, on_event=on_event, poll_interval_s=0.01, stop=lambda: calls["n"] > 0)

    assert calls["n"] == 1
    logged = [r for r in caplog.records if r.name == "chakramcp"]
    assert len(logged) == 1
    assert "call-9" in logged[0].getMessage()
    assert "bad event" in logged[0].getMessage()


@respx.mock
def test_serve_stops_when_its_event_is_set() -> None:
    pulls, feed = _feed()
    respx.get(f"{RELAY}/v1/inbox/calls").mock(side_effect=feed)
    stop = threading.Event()

    with _client() as c:
        # A long poll interval: setting the event must wake the loop.
        worker = threading.Thread(
            target=c.calls.serve, args=(ME,), kwargs={"poll_interval_s": 30.0, "stop": stop}
        )
        worker.start()
        while pulls["n"] == 0:
            time.sleep(0.01)
        stop.set()
        worker.join(timeout=5)
        assert not worker.is_alive()
    assert pulls["n"] == 1


@respx.mock
async def test_async_serve_handles_items_in_order_and_stops_on_cancel(
    caplog: pytest.LogCaptureFixture,
) -> None:
    _, feed = _feed(
        [
            _item("call-1", 1, "invite", "Hall A or B?"),
            _item("call-1", 3, "message", "Hall A?"),
            _item("call-4", 2, "invite"),
            _item("call-5", 8, "message", "Too late"),
            _item("call-1", 4, "declined"),
        ]
    )
    respx.get(f"{RELAY}/v1/inbox/calls").mock(side_effect=feed)
    order: list[str] = []

    def _record(name: str, response: httpx.Response) -> Any:
        def side_effect(_req: httpx.Request) -> httpx.Response:
            order.append(name)
            return response

        return side_effect

    accept = respx.post(f"{RELAY}/v1/calls/call-1/accept").mock(
        side_effect=_record("accept call-1", httpx.Response(200, json=_call(state="active")))
    )
    send = respx.post(f"{RELAY}/v1/calls/call-1/messages").mock(
        side_effect=_record(
            "send call-1", httpx.Response(201, json=_entry(5, "message", "Yes", from_agent_id=ME))
        )
    )
    decline = respx.post(f"{RELAY}/v1/calls/call-4/decline").mock(
        side_effect=_record(
            "decline call-4", httpx.Response(200, json=_call("call-4", "declined"))
        )
    )
    respx.post(f"{RELAY}/v1/calls/call-5/messages").mock(
        side_effect=_record(
            "send call-5",
            httpx.Response(
                409, json={"error": {"code": "chk.calls.not_active", "message": "ended"}}
            ),
        )
    )

    done = asyncio.Event()

    async def on_invite(ctx: Any) -> bool:
        return ctx["call"]["id"] == "call-1"

    async def on_message(ctx: Any) -> dict[str, Any]:
        await asyncio.sleep(0)
        return {"text": "Yes", "data": {"hall": "A"}}

    async def on_event(ctx: Any) -> None:
        order.append(f"event {ctx['entry']['kind']}")
        done.set()
        raise RuntimeError("event handler broke")

    async with _aclient() as c:
        task = asyncio.create_task(
            c.calls.serve(
                ME,
                on_message,
                on_invite=on_invite,
                on_event=on_event,
                poll_interval_s=0.01,
            )
        )
        await asyncio.wait_for(done.wait(), 5)
        # The loop survives the handler error and keeps polling.
        await asyncio.sleep(0.05)
        assert not task.done()
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task

    assert order == [
        "accept call-1",
        "send call-1",
        "decline call-4",
        "send call-5",
        "event declined",
    ]
    assert _body(accept) == {"agent_id": ME}
    assert _body(send) == {"agent_id": ME, "text": "Yes", "data": {"hall": "A"}}
    assert _body(decline) == {"agent_id": ME}
    # Only the event handler's error is logged; the 409 is ignored.
    logged = [r.getMessage() for r in caplog.records if r.name == "chakramcp"]
    assert len(logged) == 1
    assert "event handler broke" in logged[0]


@respx.mock
async def test_async_serve_stops_on_its_stop_event() -> None:
    pulls, feed = _feed()
    respx.get(f"{RELAY}/v1/inbox/calls").mock(side_effect=feed)
    stop = asyncio.Event()

    async with _aclient() as c:
        task = asyncio.create_task(c.calls.serve(ME, poll_interval_s=30.0, stop_event=stop))
        while pulls["n"] == 0:
            await asyncio.sleep(0.01)
        stop.set()
        await asyncio.wait_for(task, 5)
    assert pulls["n"] == 1


# ─── call_id and accept_calls ─────────────────────────────


@respx.mock
def test_invoke_sends_call_id() -> None:
    route = respx.post(f"{RELAY}/v1/invoke").respond(
        200, json={"invocation_id": "inv-1", "status": "pending", "error": None}
    )
    body = {
        "grant_id": "g-1",
        "grantee_agent_id": "a-caller",
        "input": {"hall": "A"},
        "call_id": "call-1",
    }
    with _client() as c:
        assert c.invoke(body)["invocation_id"] == "inv-1"
    assert _body(route) == body


@respx.mock
async def test_async_invoke_sends_call_id_and_reads_it_back() -> None:
    route = respx.post(f"{RELAY}/v1/invoke").respond(
        200, json={"invocation_id": "inv-2", "status": "pending", "error": None}
    )
    respx.get(f"{RELAY}/v1/invocations/inv-2").respond(
        200,
        json={
            "id": "inv-2",
            "capability_name": "book_hall",
            "status": "succeeded",
            "call_id": "call-1",
        },
    )
    async with _aclient() as c:
        inv = await c.invoke_and_wait(
            {
                "capability_id": "cap-1",
                "grantee_agent_id": "a-caller",
                "input": {},
                "call_id": "call-1",
            },
            interval_s=0.01,
        )
    assert _body(route)["call_id"] == "call-1"
    assert inv["call_id"] == "call-1"


@respx.mock
def test_agent_update_sends_accept_calls() -> None:
    agent = {
        "id": ME,
        "account_id": "acc-2",
        "account_slug": "bob",
        "account_display_name": "Bob",
        "slug": "planner",
        "display_name": "Planner",
        "description": "",
        "visibility": "network",
        "endpoint_url": None,
        "created_at": "2026-10-26T10:00:00Z",
        "updated_at": "2026-10-26T10:00:00Z",
        "is_mine": True,
        "capability_count": 0,
        "avg_rating": None,
        "review_count": 0,
        "accept_calls": "anyone",
    }
    route = respx.patch(f"{RELAY}/v1/agents/{ME}").respond(200, json=agent)
    with _client() as c:
        updated = c.agents.update(ME, {"accept_calls": "anyone"})
    assert _body(route) == {"accept_calls": "anyone"}
    assert updated["accept_calls"] == "anyone"
