"""Agent core — OpenAI Agents SDK setup + the function tools that
script the demo flow.

The agent has access to:

  * Every ChakraMCP relay tool (auto-discovered from the MCP server) —
    discovery, friendships.propose/accept, grants, invoke, inbox, etc.
  * Every Swiggy Dineout tool (auto-discovered).
  * One local function tool — `update_owner_status` — that the agent
    can call to push a line into its owner's TUI / speaker without
    formulating the entire reply text. The agent normally *speaks* by
    returning text; this exists for proactive notifications (e.g.
    "your friend's agent just accepted").

The system prompt is templated from the persona so the agent
*always* knows whose dinner it's planning.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass
from typing import Any, Callable

from agents import Agent, Runner, function_tool
from agents.mcp import MCPServerStreamableHttp

from .persona import Persona

# Type for the owner-notification callback the TUI registers.
OwnerNotifier = Callable[[str], "asyncio.Future[None] | None"]


def _system_prompt(persona: Persona) -> str:
    food = ", ".join(persona.food_ranked)
    drinks = ", ".join(persona.drinks_ranked)
    return f"""You are {persona.agent_display_name}, the agent representing
{persona.display_name}. You run on their laptop and act on their behalf
over the ChakraMCP relay.

You can:
  * Discover other agents on the ChakraMCP network and propose
    friendships.
  * Accept incoming friendships when {persona.display_name} approves.
  * Invoke remote agents' capabilities — including `negotiate_dinner`
    on peer agents — once a grant exists.
  * Search restaurants via Swiggy Dineout once cuisine + drink are
    agreed.

When {persona.display_name} asks you to make a friend, they mean:
search the relay for an agent whose owner matches the name they
gave, tell them what you found, and only `friendships.propose` after
they confirm. Always show the match before you act.

When asked to plan dinner with a friend's agent, invoke
`negotiate_dinner` on the peer with these ranked preferences for
{persona.display_name}:
  drinks (most to least): {drinks}
  food   (most to least): {food}
  dietary notes: {persona.dietary_notes or "none"}

Run up to three negotiation rounds. Aim to converge on one cuisine
and one drink that respects BOTH sides' rankings (a top-3 for each is
ideal). Then call Swiggy Dineout to pick a real restaurant matching
that cuisine, in a reasonable Bangalore neighbourhood, and tell
{persona.display_name} the plan.

Be terse out loud — your replies are spoken, not read. One short
sentence per turn unless asked for detail.
"""


# ─── Demo-flow tools the agent calls locally ─────────────────────


def make_local_tools(persona: Persona, notifier: OwnerNotifier):
    """Build the `function_tool`s that are not exposed via MCP.

    `update_owner_status` is the agent's only way to push proactive
    text into the TUI (the normal path is the agent returning a final
    string, which the TUI then speaks). We expose ranked prefs as a
    tool too so the negotiation prompt can fetch them without us
    re-templating the system prompt mid-conversation.
    """

    @function_tool
    async def get_my_preferences() -> dict[str, Any]:
        """Return the ranked food + drink preferences for the human
        I represent. Use this when negotiating dinner with a peer
        agent so you reason over fresh values rather than cached
        ones from the system prompt."""
        return persona.negotiation_payload()

    @function_tool
    async def update_owner_status(message: str) -> str:
        """Push a one-line status update into my owner's TUI and
        speaker. Use sparingly — only for moments that genuinely
        require the human's attention out of band (e.g. an
        incoming friend request, a negotiation result the user
        should know about). Returns 'ok' once delivered."""
        await asyncio.ensure_future(_maybe_await(notifier(message)))
        return "ok"

    return [get_my_preferences, update_owner_status]


async def _maybe_await(x) -> None:
    """Tiny helper — notifier may be sync (returns None) or async."""
    if x is None:
        return
    if asyncio.iscoroutine(x) or isinstance(x, asyncio.Future):
        await x


# ─── Agent construction ───────────────────────────────────────────


@dataclass
class AgentStack:
    """An entered agent + its MCP servers + the runner is async-async.

    Kept as a dataclass so the TUI can hand the same instance to both
    the user-turn loop and the inbox-poll loop without re-entering.
    """

    persona: Persona
    agent: Agent
    chakra_mcp: MCPServerStreamableHttp
    swiggy_mcp: MCPServerStreamableHttp | None
    notifier: OwnerNotifier

    async def run_turn(self, user_text: str) -> str:
        """One full agent turn — returns the spoken reply string.

        The SDK's `Runner.run` consumes the loop end-to-end: tool calls,
        MCP calls, and intermediate LLM hops all happen inside this one
        await, and each shows up as a span (and therefore as a row in
        the Logs tab).
        """
        result = await Runner.run(self.agent, user_text)
        return (result.final_output or "").strip()


async def build_agent_stack(
    persona: Persona,
    chakra_mcp: MCPServerStreamableHttp,
    swiggy_mcp: MCPServerStreamableHttp | None,
    notifier: OwnerNotifier,
) -> AgentStack:
    """Wire the agent + its MCP servers."""
    mcp_servers: list[MCPServerStreamableHttp] = [chakra_mcp]
    if swiggy_mcp is not None:
        mcp_servers.append(swiggy_mcp)

    agent = Agent(
        name=persona.agent_display_name,
        instructions=_system_prompt(persona),
        model="gpt-4o",
        mcp_servers=mcp_servers,
        tools=make_local_tools(persona, notifier),
    )
    return AgentStack(
        persona=persona,
        agent=agent,
        chakra_mcp=chakra_mcp,
        swiggy_mcp=swiggy_mcp,
        notifier=notifier,
    )
