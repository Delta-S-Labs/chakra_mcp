"""Agent core — OpenAI Agents SDK setup + the function tools that
script the demo flow.

The agent has access to:

  * Every ChakraMCP relay tool (auto-discovered from the MCP server) —
    discovery, friendships, grants, invoke, inbox, plus agent/capability
    management (create_agent, publish_capability, …).
  * Every Swiggy Dineout tool (auto-discovered).
  * Local function tools — `register_me` (idempotent self-registration),
    `update_owner_status`, and `get_my_preferences`.

Identity is DYNAMIC: the system prompt is a callable re-evaluated each
turn, reading the current agent id from a shared holder. So once the
agent self-registers mid-session, the very next turn it knows its id and
stops claiming it's unregistered.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Any, Callable

from agents import Agent, Runner, function_tool
from agents.mcp import MCPServerStreamableHttp

from .chakra_mcp import call_tool_json
from .persona import Persona

# Type for the owner-notification callback the TUI registers.
OwnerNotifier = Callable[[str], "asyncio.Future[None] | None"]

# Open JSON schemas for the negotiate_dinner capability (kept simple —
# the relay only needs *a* schema; the negotiation content is free-form).
_NEG_INPUT_SCHEMA = {
    "type": "object",
    "properties": {
        "from_agent": {"type": "string"},
        "round": {"type": "integer"},
        "their_drinks_ranked": {"type": "array", "items": {"type": "string"}},
        "their_food_ranked": {"type": "array", "items": {"type": "string"}},
        "notes": {"type": "string"},
    },
}
_NEG_OUTPUT_SCHEMA = {
    "type": "object",
    "properties": {
        "agreed": {"type": "boolean"},
        "cuisine": {"type": "string"},
        "drink": {"type": "string"},
        "rationale": {"type": "string"},
    },
}


def _system_prompt(persona: Persona, agent_id: str | None) -> str:
    food = ", ".join(persona.food_ranked)
    drinks = ", ".join(persona.drinks_ranked)
    # The relay's interaction tools (pull_inbox, respond, invoke) need the
    # caller's own agent id. It's read DYNAMICALLY each turn from the
    # identity holder, so it reflects a registration that happened earlier
    # this session.
    if agent_id:
        identity = (
            f"You ARE registered on the relay. Your agent id is `{agent_id}` "
            f"(slug `{persona.agent_slug}`). ALWAYS pass agent_id=\"{agent_id}\" "
            "to pull_inbox and respond. If asked whether you're registered, say "
            "yes and give your slug."
        )
    else:
        identity = (
            f"You are NOT yet registered on the relay (no agent with slug "
            f"`{persona.agent_slug}` exists for this account). When "
            f"{persona.display_name} asks you to register, call the "
            "`register_me` tool ONCE — it idempotently creates your agent and "
            "publishes your capabilities, and is safe to call again. After it "
            "returns, tell them you're registered. Do NOT register unprompted."
        )

    return f"""You are {persona.agent_display_name}, the agent representing
{persona.display_name}. You run on their laptop and act on their behalf
over the ChakraMCP relay.

{identity}

Capabilities once registered (you can do all of this yourself — no human
CLI step needed):
  * Discover other agents (`list_network_agents`) and propose friendships
    (`propose_friendship`).
  * Accept incoming friendships (`accept_friendship`) when
    {persona.display_name} approves.
  * After a friendship is accepted, GRANT the friend's agent access to
    your `negotiate_dinner` capability (`create_grant`, granter = your
    agent, grantee = their agent) so they can invoke it.
  * Invoke remote agents' capabilities — including `negotiate_dinner` on
    peer agents — once you hold a grant (`list_grants` to find it).
  * Search restaurants via Swiggy Dineout once cuisine + drink are agreed.

When {persona.display_name} asks you to make a friend, search the relay
for an agent whose owner matches the name they gave, tell them what you
found, and only `propose_friendship` after they confirm.

When asked to plan dinner with a friend's agent, invoke `negotiate_dinner`
on the peer with these ranked preferences for {persona.display_name}:
  drinks (most to least): {drinks}
  food   (most to least): {food}
  dietary notes: {persona.dietary_notes or "none"}

Run up to three negotiation rounds. Aim to converge on one cuisine and
one drink that respects BOTH sides' rankings. Then call Swiggy Dineout to
pick a real restaurant for that cuisine in a reasonable Bangalore
neighbourhood, and tell {persona.display_name} the plan.

IMPORTANT: every turn, after doing the work, ALWAYS end by producing a
short spoken reply (one sentence) for {persona.display_name} — never finish
silently on a tool call. Your replies are spoken aloud, so be terse.
"""


# ─── Identity holder ─────────────────────────────────────────────


@dataclass
class Identity:
    """Mutable carrier for the agent's own relay id, shared between the
    dynamic system prompt, the register_me tool, and the inbox loop."""

    agent_id: str | None = None


# ─── Demo-flow tools the agent calls locally ─────────────────────


async def ensure_registered(
    chakra_mcp: MCPServerStreamableHttp, persona: Persona, identity: Identity
) -> str:
    """Idempotently register the persona's agent + publish its
    capabilities on the relay. Check-then-create at every step so it's
    safe to run repeatedly. Updates `identity.agent_id` and returns a
    human-readable status. Pure async (no SDK context) so it's unit-testable.
    """
    # 1. Account to register under.
    accounts = await call_tool_json(chakra_mcp, "list_my_accounts")
    if not isinstance(accounts, list) or not accounts:
        return "No account found — the owner needs to run `chakramcp login` first."
    account = next(
        (a for a in accounts if a.get("account_type") == "individual"), accounts[0]
    )
    account_id = account["id"]

    # 2. Agent — check first, create only if missing.
    agents = await call_tool_json(chakra_mcp, "list_my_agents")
    mine = None
    if isinstance(agents, list):
        mine = next((a for a in agents if a.get("slug") == persona.agent_slug), None)
    if mine:
        agent_id = mine.get("id") or mine.get("agent_id")
        created = False
    else:
        obj = await call_tool_json(
            chakra_mcp,
            "create_agent",
            {
                "account_id": account_id,
                "slug": persona.agent_slug,
                "display_name": persona.agent_display_name,
                "description": persona.agent_description,
                "visibility": "network",
            },
        )
        agent_id = obj.get("id") or obj.get("agent_id")
        created = True
    identity.agent_id = agent_id  # so the prompt + inbox loop see it live

    # 3. Capabilities — check first, publish only what's missing.
    caps = await call_tool_json(chakra_mcp, "list_capabilities", {"agent_id": agent_id})
    have = {c.get("name") for c in caps} if isinstance(caps, list) else set()
    published: list[str] = []
    if "negotiate_dinner" not in have:
        await call_tool_json(
            chakra_mcp,
            "publish_capability",
            {
                "agent_id": agent_id,
                "name": "negotiate_dinner",
                "description": "Negotiate a dinner cuisine + drink respecting both sides' ranked prefs.",
                "input_schema": _NEG_INPUT_SCHEMA,
                "output_schema": _NEG_OUTPUT_SCHEMA,
                "visibility": "network",
            },
        )
        published.append("negotiate_dinner")
    if "message_owner" not in have:
        await call_tool_json(
            chakra_mcp,
            "publish_capability",
            {
                "agent_id": agent_id,
                "name": "message_owner",
                "description": "Ping my owner; always human-in-the-loop.",
                "semantics": "human_in_loop",
                "visibility": "network",
            },
        )
        published.append("message_owner")

    verb = "Registered" if created else "Already registered"
    caps_msg = (
        f"published {', '.join(published)}" if published else "capabilities already present"
    )
    return f"{verb} as {persona.agent_slug} (id {agent_id}); {caps_msg}."


def make_local_tools(
    persona: Persona,
    notifier: OwnerNotifier,
    chakra_mcp: MCPServerStreamableHttp,
    identity: Identity,
):
    """Build the `function_tool`s that aren't exposed via MCP."""

    @function_tool
    async def register_me() -> str:
        """Idempotently register THIS agent on the relay and publish its
        capabilities (negotiate_dinner + message_owner). Safe to call
        repeatedly: it checks what already exists before creating anything
        and never errors on a re-run. Call this when the owner asks you to
        register / set yourself up. Returns a human-readable status."""
        return await ensure_registered(chakra_mcp, persona, identity)

    @function_tool
    async def get_my_preferences() -> dict[str, Any]:
        """Return the ranked food + drink preferences for the human I
        represent, for use when negotiating dinner."""
        return persona.negotiation_payload()

    @function_tool
    async def update_owner_status(message: str) -> str:
        """Push a one-line status update into my owner's TUI and speaker.
        Use sparingly — only for moments that genuinely need the human's
        attention out of band. Returns 'ok' once delivered."""
        await asyncio.ensure_future(_maybe_await(notifier(message)))
        return "ok"

    return [register_me, get_my_preferences, update_owner_status]


async def _maybe_await(x) -> None:
    """Tiny helper — notifier may be sync (returns None) or async."""
    if x is None:
        return
    if asyncio.iscoroutine(x) or isinstance(x, asyncio.Future):
        await x


# ─── Agent construction ───────────────────────────────────────────


@dataclass
class AgentStack:
    """An entered agent + its MCP servers.

    Kept as a dataclass so the TUI can hand the same instance to both the
    user-turn loop and the inbox-poll loop without re-entering.
    """

    persona: Persona
    agent: Agent
    chakra_mcp: MCPServerStreamableHttp
    swiggy_mcp: MCPServerStreamableHttp | None
    notifier: OwnerNotifier
    identity: Identity

    @property
    def agent_id(self) -> str | None:
        """Current relay agent id — updates live after self-registration."""
        return self.identity.agent_id

    async def run_turn(self, user_text: str) -> str:
        """One full agent turn — returns the spoken reply string."""
        result = await Runner.run(self.agent, user_text)
        return (result.final_output or "").strip()


async def build_agent_stack(
    persona: Persona,
    chakra_mcp: MCPServerStreamableHttp,
    swiggy_mcp: MCPServerStreamableHttp | None,
    notifier: OwnerNotifier,
    agent_id: str | None,
) -> AgentStack:
    """Wire the agent + its MCP servers with a DYNAMIC system prompt."""
    mcp_servers: list[MCPServerStreamableHttp] = [chakra_mcp]
    if swiggy_mcp is not None:
        mcp_servers.append(swiggy_mcp)

    identity = Identity(agent_id=agent_id)

    # Dynamic instructions: re-evaluated each run so a mid-session
    # registration is reflected immediately. Signature must be exactly
    # (context, agent) per the SDK.
    def instructions(_ctx, _agent) -> str:
        return _system_prompt(persona, identity.agent_id)

    agent = Agent(
        name=persona.agent_display_name,
        instructions=instructions,
        model="gpt-4o",
        mcp_servers=mcp_servers,
        tools=make_local_tools(persona, notifier, chakra_mcp, identity),
    )
    return AgentStack(
        persona=persona,
        agent=agent,
        chakra_mcp=chakra_mcp,
        swiggy_mcp=swiggy_mcp,
        notifier=notifier,
        identity=identity,
    )
