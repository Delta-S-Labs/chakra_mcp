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
import os
from dataclasses import dataclass, field
from typing import Any, Callable

from agents import Agent, OpenAIChatCompletionsModel, Runner, function_tool
from agents.mcp import MCPServerStreamableHttp

from .chakra_mcp import call_tool_json
from .persona import Persona

# Provider selection via .env: LLM_PROVIDER = "openai" (default) | "groq".
# Defaults per provider, overridable with OPENAI_MODEL / GROQ_MODEL.
DEFAULT_OPENAI_MODEL = "gpt-5-mini"
DEFAULT_GROQ_MODEL = "llama-3.3-70b-versatile"
GROQ_BASE_URL = "https://api.groq.com/openai/v1"


def _provider() -> str:
    return (os.environ.get("LLM_PROVIDER") or "openai").strip().lower()


def llm_label() -> str:
    """Human-readable provider/model, for the startup banner."""
    if _provider() == "groq":
        model = os.environ.get("GROQ_MODEL", DEFAULT_GROQ_MODEL).strip() or DEFAULT_GROQ_MODEL
        return f"groq / {model}"
    model = os.environ.get("OPENAI_MODEL", DEFAULT_OPENAI_MODEL).strip() or DEFAULT_OPENAI_MODEL
    return f"openai / {model}"


def _build_model():
    """Resolve the Agent's `model`.

    - openai: return the model id as a string; the SDK uses its default
      OpenAI client (OPENAI_API_KEY).
    - groq: Groq is OpenAI-API-compatible, so we point an AsyncOpenAI
      client at Groq's base URL with GROQ_API_KEY and wrap it in a
      Chat-Completions model (Groq doesn't implement the Responses API).
    """
    if _provider() == "groq":
        # Imported lazily so the openai client is only constructed when
        # groq is actually selected.
        from openai import AsyncOpenAI

        key = os.environ.get("GROQ_API_KEY")
        if not key:
            raise RuntimeError("LLM_PROVIDER=groq but GROQ_API_KEY is not set")
        model = os.environ.get("GROQ_MODEL", DEFAULT_GROQ_MODEL).strip() or DEFAULT_GROQ_MODEL
        client = AsyncOpenAI(base_url=GROQ_BASE_URL, api_key=key)
        return OpenAIChatCompletionsModel(model=model, openai_client=client)

    return os.environ.get("OPENAI_MODEL", DEFAULT_OPENAI_MODEL).strip() or DEFAULT_OPENAI_MODEL


# Sliding-window memory: keep the most recent N conversation TURNS for the
# session. Windowing by whole turns (a turn = a user message plus the
# assistant/tool-call/tool-output items that follow it) prevents context
# bloat without orphaning a tool_call from its tool_output — slicing by raw
# item count can split that pair and make the model API error.
MAX_HISTORY_TURNS = 16


def _is_user_message(item: Any) -> bool:
    return isinstance(item, dict) and item.get("role") == "user"


def _trim_history(items: list, max_turns: int = MAX_HISTORY_TURNS) -> list:
    """Drop the oldest complete turns, keeping the last `max_turns`."""
    turns: list[list] = []
    current: list = []
    for it in items:
        if _is_user_message(it) and current:
            turns.append(current)
            current = []
        current.append(it)
    if current:
        turns.append(current)
    kept = turns[-max_turns:]
    return [it for turn in kept for it in turn]

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
            "yes and give your slug. If asked to register, set yourself up, or "
            "publish/check your capabilities, call the `register_me` tool — it's "
            "idempotent and will publish any missing capabilities."
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

TOOL MAP — use the RIGHT tool (this matters):
  * Incoming FRIEND REQUESTS: `list_friendships` with
    direction="inbound", status="proposed". This is the ONLY way to see
    pending friend requests. Do NOT use pull_inbox for friend requests.
    To accept one, call `accept_friendship` with its `friendship_id`
    (from that list); to decline, `reject_friendship`.
  * `pull_inbox` is ONLY for pending capability INVOCATIONS (e.g. a peer
    calling your negotiate_dinner) — never friend requests.
  * Discover other agents: `list_network_agents`. Propose a friendship:
    `propose_friendship`.
  * After a friendship is accepted, GRANT the friend's agent access to
    your `negotiate_dinner` capability (`create_grant`, granter = your
    agent, grantee = their agent) so they can invoke it.
  * Invoke remote agents' capabilities — including `negotiate_dinner` on
    peer agents — once you hold a grant (`list_grants` to find it).
  * Search restaurants via Swiggy Dineout once cuisine + drink are agreed.

When {persona.display_name} asks "do I have any friend requests?" or to
accept one, ALWAYS call `list_friendships(direction="inbound",
status="proposed")` first and report exactly what it returns — never
claim there are none without calling it.

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

    # 3. Capabilities — publish any that are missing (idempotent).
    published = await ensure_capabilities(chakra_mcp, agent_id)

    verb = "Registered" if created else "Already registered"
    caps_msg = (
        f"published {', '.join(published)}" if published else "capabilities already present"
    )
    return f"{verb} as {persona.agent_slug} (id {agent_id}); {caps_msg}."


async def ensure_capabilities(
    chakra_mcp: MCPServerStreamableHttp, agent_id: str
) -> list[str]:
    """Publish the demo capabilities the agent is missing. Idempotent —
    checks `list_capabilities` first and only publishes the gaps. Returns
    the names it newly published (empty if all were already there)."""
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
    return published


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
    # Rolling conversation history for the USER dialogue, so the agent
    # remembers context across turns ("yes, send it" → which agent?). The
    # background inbox poll runs with remember=False so it never pollutes
    # this thread.
    history: list = field(default_factory=list)

    @property
    def agent_id(self) -> str | None:
        """Current relay agent id — updates live after self-registration."""
        return self.identity.agent_id

    async def run_turn(self, user_text: str, *, remember: bool = True) -> str:
        """One full agent turn — returns a non-empty spoken reply string.

        With remember=True the turn is threaded onto the rolling history so
        the agent has conversational memory. The model sometimes ends a
        turn on tool calls without a final sentence, so we fall back to the
        concatenated message text, then to a tool-call summary.
        """
        if remember:
            agent_input: Any = [*self.history, {"role": "user", "content": user_text}]
        else:
            agent_input = user_text
        result = await Runner.run(self.agent, agent_input, max_turns=12)
        if remember:
            # Keep the full thread (incl. tool calls/results) but window it
            # to the last N turns so the session remembers context without
            # the prompt growing unbounded — trimmed at turn boundaries so
            # tool_call/tool_output pairs are never split.
            self.history = _trim_history(result.to_input_list())

        out = result.final_output
        text = out.strip() if isinstance(out, str) else ("" if out is None else str(out).strip())
        if text:
            return text

        # Fallback 1: concatenate any assistant message text in the run.
        try:
            from agents.items import ItemHelpers

            text = (ItemHelpers.text_message_outputs(result.new_items) or "").strip()
        except Exception:
            text = ""
        if text:
            return text

        # Fallback 2: name the tools it invoked so the turn isn't silent.
        names: list[str] = []
        for item in getattr(result, "new_items", []) or []:
            raw = getattr(item, "raw_item", None)
            name = getattr(raw, "name", None)
            if name:
                names.append(name)
        if names:
            uniq = list(dict.fromkeys(names))
            return "Done — I called: " + ", ".join(uniq) + "."
        return "Okay."


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
        model=_build_model(),
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
