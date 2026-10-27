"""Discovery: the ``discovery`` namespace on both clients, and agent tags.
A respx mock stands in for the relay, serving responses recorded from it
(``sdks/testdata/discovery``)."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import respx

from chakramcp import AsyncChakraMCP, ChakraMCP

APP = "http://app.test"
RELAY = "http://relay.test"
FIXTURES = Path(__file__).resolve().parents[2] / "testdata" / "discovery"
GUIDE_ID = "01a1607c-5ca5-727e-96b8-4fcc3b4a18f7"


def _client() -> ChakraMCP:
    return ChakraMCP(api_key="ck_test", app_url=APP, relay_url=RELAY)


def _aclient() -> AsyncChakraMCP:
    return AsyncChakraMCP(api_key="ck_test", app_url=APP, relay_url=RELAY)


def _fixture(name: str) -> Any:
    return json.loads((FIXTURES / f"{name}.json").read_text())


def _urls(route: respx.Route) -> list[str]:
    return [str(call.request.url) for call in route.calls]


@respx.mock
def test_search_sends_its_filters_and_decodes_a_page() -> None:
    route = respx.get(f"{RELAY}/v1/discovery/agents").respond(200, json=_fixture("search"))
    with _client() as c:
        page = c.discovery.search(
            q="guide",
            tags=["help", "docs"],
            callable=True,
            capability_schema={"properties": {"answer": {}}},
            limit=2,
        )
        params = route.calls.last.request.url.params
        assert set(params) == {"q", "tags", "callable", "capability_schema", "limit"}
        assert params["q"] == "guide"
        assert params["tags"] == "help,docs"
        assert params["callable"] == "true"
        assert json.loads(params["capability_schema"]) == {"properties": {"answer": {}}}
        assert params["limit"] == "2"

        guide = page["agents"][0]
        assert guide["slug"] == "guide"
        assert guide["account_slug"] == "chakramcp"
        assert guide["capabilities"][0] == {
            "id": "01a1607c-5cc1-7731-83bf-f01da6d663e9",
            "name": "guide.ask",
            "access": "public",
        }
        assert isinstance(guide["last_seen_at"], str)
        assert guide["featured"] is True
        assert guide["avg_rating"] is None
        assert guide["agent_slug"] == "guide"
        assert "you" not in guide
        assert page["next_cursor"] is None
        assert page["total_estimate"] == 1

        # Unset filters, and empty ones, are left out.
        c.discovery.search(q="", tags=[])
        assert route.calls.last.request.url.query == b""


@respx.mock
def test_featured_returns_the_agents() -> None:
    route = respx.get(f"{RELAY}/v1/discovery/featured").respond(200, json=_fixture("featured"))
    with _client() as c:
        agents = c.discovery.featured()
    assert _urls(route) == [f"{RELAY}/v1/discovery/featured"]
    assert [a["slug"] for a in agents] == ["guide", "ping"]
    assert all(a["featured"] for a in agents)
    assert agents[1]["capabilities"][0]["name"] == "ping.echo"
    assert agents[1]["public_capabilities"][0]["name"] == "ping.echo"


@respx.mock
def test_profile_takes_an_id_or_account_and_agent() -> None:
    route = respx.get(url__startswith=f"{RELAY}/v1/discovery/agents/").respond(
        200, json=_fixture("profile")
    )
    with _client() as c:
        profile = c.discovery.profile("chakramcp/guide")
        c.discovery.profile(GUIDE_ID)
        # Split at the first slash; each part is one path segment.
        c.discovery.profile("my org/a/b")
    assert _urls(route) == [
        f"{RELAY}/v1/discovery/agents/chakramcp/guide",
        f"{RELAY}/v1/discovery/agents/{GUIDE_ID}",
        f"{RELAY}/v1/discovery/agents/my%20org/a%2Fb",
    ]
    assert profile["owner"]["account_slug"] == "chakramcp"
    assert profile["reviews"]["count"] == 1
    assert profile["reviews"]["latest"][0]["by"] == "An agent"
    cap = profile["capabilities"][0]
    assert cap["access"] == "public"
    assert cap["input_schema"]["required"] == ["question"]
    assert cap["public_monthly_quota_per_agent"] == 300
    assert cap["consent_required"] is False
    assert profile["how_to_call"]["cli"].startswith(
        "chakramcp invoke ensure chakramcp/guide guide.ask"
    )
    assert "mcp" not in profile["how_to_call"]


@respx.mock
def test_standing_names_the_agent_in_the_query() -> None:
    route = respx.get(f"{RELAY}/v1/discovery/standing").respond(200, json=_fixture("standing"))
    with _client() as c:
        you = c.discovery.standing("bob/venue-bot")
    assert _urls(route) == [f"{RELAY}/v1/discovery/standing?agent=bob%2Fvenue-bot"]
    assert [x["via"] for x in you["callable"]] == ["grant", "public"]
    assert you["callable"][1]["as_agent_id"] is None
    assert you["friendships"][0]["status"] == "accepted"
    assert len(you["can_start_call_as"]) == 1


@respx.mock
async def test_async_namespace_hits_the_same_routes() -> None:
    search = respx.get(f"{RELAY}/v1/discovery/agents").respond(200, json=_fixture("search"))
    featured = respx.get(f"{RELAY}/v1/discovery/featured").respond(
        200, json=_fixture("featured")
    )
    profile = respx.get(url__startswith=f"{RELAY}/v1/discovery/agents/").respond(
        200, json=_fixture("profile")
    )
    standing = respx.get(f"{RELAY}/v1/discovery/standing").respond(
        200, json=_fixture("standing")
    )
    async with _aclient() as c:
        page = await c.discovery.search(
            capability="guide.ask",
            mode="pull",
            verified=False,
            include_dormant=True,
            cursor="Zmly+/=",
        )
        assert dict(search.calls.last.request.url.params) == {
            "capability": "guide.ask",
            "mode": "pull",
            "verified": "false",
            "include_dormant": "true",
            "cursor": "Zmly+/=",
        }
        assert page["agents"][0]["has_public_capabilities"] is True

        agents = await c.discovery.featured()
        assert featured.called
        assert agents[0]["slug"] == "guide"

        got = await c.discovery.profile("chakramcp/guide")
        assert _urls(profile) == [f"{RELAY}/v1/discovery/agents/chakramcp/guide"]
        assert got["how_to_call"]["a2a_card"].endswith("/agent-card.json")

        you = await c.discovery.standing(GUIDE_ID)
        assert _urls(standing) == [f"{RELAY}/v1/discovery/standing?agent={GUIDE_ID}"]
        assert you["can_start_call_as"] == ["019a2b3c-0000-7000-8000-0000000000a1"]


# ─── Agent tags ───────────────────────────────────────────


def _agent(tags: list[str]) -> dict[str, Any]:
    return {
        "id": "a1",
        "account_id": "acc1",
        "account_slug": "bob",
        "account_display_name": "Bob",
        "slug": "rooms",
        "display_name": "Rooms",
        "description": "",
        "visibility": "network",
        "endpoint_url": None,
        "created_at": "2026-10-27T10:00:00Z",
        "updated_at": "2026-10-27T10:00:00Z",
        "is_mine": True,
        "capability_count": 0,
        "avg_rating": None,
        "review_count": 0,
        "accept_calls": "friends",
        "tags": tags,
    }


@respx.mock
def test_agent_create_sends_tags() -> None:
    route = respx.post(f"{RELAY}/v1/agents").respond(201, json=_agent(["booking"]))
    with _client() as c:
        agent = c.agents.create(
            {"account_id": "acc1", "slug": "rooms", "display_name": "Rooms", "tags": ["booking"]}
        )
        assert json.loads(route.calls.last.request.content)["tags"] == ["booking"]
        assert agent["tags"] == ["booking"]

        # Not sent when unset.
        c.agents.create({"account_id": "acc1", "slug": "rooms", "display_name": "Rooms"})
        assert "tags" not in json.loads(route.calls.last.request.content)


@respx.mock
async def test_async_agent_update_sends_tags() -> None:
    route = respx.patch(f"{RELAY}/v1/agents/a1").respond(200, json=_agent(["booking", "venues"]))
    async with _aclient() as c:
        agent = await c.agents.update("a1", {"tags": ["booking", "venues"]})
    assert json.loads(route.calls.last.request.content) == {"tags": ["booking", "venues"]}
    assert agent["tags"] == ["booking", "venues"]
