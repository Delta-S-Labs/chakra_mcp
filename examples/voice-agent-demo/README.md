# Voice Agent Demo — Kaustav × Aparajita

A two-laptop demo: each person runs a voice agent on their laptop. The
agents discover each other on the ChakraMCP relay, ask their owners
for permission to become friends, then **autonomously negotiate** what
cuisine and drinks Kaustav and Aparajita should go out for tonight by
exchanging ranked preferences over the relay. The winning agent
queries Swiggy Dineout MCP to pick an actual restaurant. Both agents
then tell their owners the plan.

> Built for a single-take phone-camera recording. Two terminals, two
> voices, the full agent-to-agent flow visible end-to-end.

## What you'll see on each laptop

A two-tab Textual TUI:

- **Agent** tab — the conversation: your voice transcript + the
  agent's spoken replies (streamed as captions).
- **Logs** tab — an expandable tree of *every* LLM call, tool call,
  ChakraMCP relay call, and Swiggy MCP call as it happens, with
  arguments and results. This is the camera-friendly proof that the
  agents are really negotiating, not faking it.

Push-to-talk: hold **space** while you speak, release to send.

## Demo flow (the script you'll follow on camera)

1. **Start.** Each laptop runs the demo with a persona flag:
   ```sh
   uv run voice-agent --persona kaustav     # Laptop A
   uv run voice-agent --persona aparajita   # Laptop B
   ```
2. **You** (Kaustav, holding space): *"Find Aparajita's agent and send
   her a friend request."*
   → Agent calls ChakraMCP **discovery** (visible in Logs) → speaks
   the matching agent's name back to you → you confirm → agent fires
   **`friendships.propose`**.
3. **Aparajita's agent** picks up the proposal from its inbox loop,
   surfaces it in her TUI, and *speaks to her*: *"Kaustav's agent
   wants to be friends and is requesting access to the
   `negotiate_dinner` capability — should I accept?"* She says yes →
   **`friendships.accept`** + reciprocal `negotiate_dinner` grants
   both ways.
4. **You:** *"Ask her agent what cuisine and drinks we should plan
   for tonight."* → your agent invokes `negotiate_dinner` on her
   agent, passing **your ranked food + drink prefs**. Her agent's
   LLM merges with **her ranked prefs**, counters, your agent
   counters again. Two to three visible rounds in the Logs tab until
   they converge.
5. **One agent** (the initiator) queries **Swiggy Dineout MCP** for
   a restaurant matching the agreed cuisine + drink, returns the
   pick.
6. **Both agents speak** the result to their owners: *"You're going
   to [restaurant] for [cuisine] and [drink]."*

## Stack

| Piece | Choice |
|---|---|
| Agent loop | [OpenAI Agents SDK](https://openai.github.io/openai-agents-python/) — OpenAI (gpt-5-mini) or Groq, via `LLM_PROVIDER` |
| Voice STT | Sarvam `POST /speech-to-text` |
| Voice TTS | Sarvam `POST /text-to-speech` (`bulbul:v3`) |
| ChakraMCP access | Reads the JWT minted by `chakramcp login`, opens an MCP client to `https://relay.chakramcp.com/mcp` |
| Swiggy Dineout | MCP client to `https://mcp.swiggy.com/...` (OAuth 2.1 + PKCE, browser login at startup) |
| TUI | [Textual](https://textual.textualize.io/) — 2 tabs, expandable tree for logs |
| Audio I/O | `sounddevice` + `numpy` (push-to-talk) |

The agent loop runs the OpenAI Agents SDK with **two MCP servers**
attached as tool sources (ChakraMCP + Swiggy) so every relay
invocation and every Swiggy call shows up as a tool span in the
Logs tab automatically.

## Setup (each laptop, one time)

```sh
# Prereqs: python 3.11+, uv (https://docs.astral.sh/uv/)

cd examples/voice-agent-demo
cp .env.example .env
# Fill in .env: an LLM key (OpenAI or Groq) + a Sarvam key (see file)

uv sync   # installs the deps locked in pyproject.toml

# Log in to ChakraMCP CLI — the agent will reuse this token.
chakramcp login
#   Laptop A: sign in as kaustav@chakramcp.com
#   Laptop B: sign in as aparajita@... (whatever account you use)

# Mic permission: macOS will ask the terminal for mic access on first
# space-press. Grant it once.
```

**Swiggy login happens at startup, not here.** Swiggy MCP is OAuth 2.1
(PKCE) — there's no token to paste. The *first* time you run the agent
on a laptop, it opens a browser tab for Swiggy login, then caches the
~5-day token locally (per persona, under
`~/.config/voice-agent-demo/`). Do this first run a few minutes before
filming so the browser handoff isn't on camera. Re-runs within 5 days
skip it. Leave `SWIGGY_MCP_URL` blank in `.env` to skip Swiggy
entirely (the agent will just suggest a restaurant verbally).

## Run

```sh
uv run voice-agent --persona kaustav     # Laptop A
uv run voice-agent --persona aparajita   # Laptop B
```

Add `--persona <name>` for any persona file in `personas/`. To add a
third demo persona, drop a new JSON there with the same shape.

## Persona files

Each `personas/<name>.json` carries the display name, the ChakraMCP
agent slug to register on first run, and the ranked food + drink
preferences the agent negotiates with. Edit freely — the negotiation
prompt reads them as opaque ranked lists.

```jsonc
{
  "display_name": "Kaustav",
  "agent_slug": "kaustav-voice-bot",
  "agent_display_name": "Kaustav's Dinner Bot",
  "drinks_ranked": ["beer", "cocktail", "wine"],
  "food_ranked": ["sushi", "biryani", "pizza", "thai green curry", ...]
}
```

## First run on each laptop

Two ways to register the persona's agent (do it once per laptop, after
`chakramcp login`):

**A. Ask the agent to register itself (voice).** The relay now exposes
`list_my_accounts`, `create_agent`, and `publish_capability` over MCP, so
on first run you can push-to-talk and say *"register yourself on the
relay."* The agent calls those tools live (you'll see them in the Logs
tab) and publishes `negotiate_dinner`. Requires a relay running this
version (deployed to `relay.chakramcp.com`).

**B. Run the bundled script (deterministic, good for filming prep).**

```bash
./scripts/register-agent.sh kaustav      # on Kaustav's laptop
./scripts/register-agent.sh aparajita    # on Aparajita's laptop
```

It reads `personas/<name>.json`, picks your account from
`chakramcp whoami`, registers the agent **network-visible** (so the peer
can discover it), and publishes the two capabilities the flow uses:
`message_owner` (reserved template, human-in-the-loop) and the custom
`negotiate_dinner`. Verify with `chakramcp capabilities list --agent <slug>`.

On startup the voice agent resolves its own agent id by slug and pins it
into the system prompt — that's what lets `pull_inbox`/`respond`/`invoke`
work. If no matching agent exists yet, it prints a registration reminder
and runs in voice-only degraded mode instead of erroring on every inbox
poll.

Once both laptops have registered, both `*-voice-bot` agents exist on the
relay and you can start the discovery flow.

## Troubleshooting

| Symptom | Fix |
|---|---|
| TUI says "no CLI token" | Run `chakramcp login` in the same shell first. The agent reads the active network's token from `~/Library/Application Support/com.chakramcp.chakramcp/config.toml` (or the platform equivalent). |
| Mic silent / push-to-talk no-op | Check terminal has mic permission (macOS System Settings → Privacy → Microphone). On Linux: `aplay -l` / `arecord -l` to confirm a default input. |
| Sarvam 401 | `SARVAM_API_KEY` in `.env` is wrong or expired. |
| `friendships.propose` 409 | A friendship between these two agents already exists — accept it from her side or skip step 2. |
| `negotiate_dinner` not found | The peer agent hasn't published it yet. Run `./scripts/register-agent.sh <persona>` on that laptop (see "First run" above). |
| Bot says "error checking inbox" / `missing field agent_id` | No agent is registered for this account, so there's no inbox to pull. Run `./scripts/register-agent.sh <persona>`, then restart the agent. |
| Swiggy browser tab didn't open | The terminal prints the auth URL — open it manually. Headless box? Run the first auth on a machine with a browser; the cached token under `~/.config/voice-agent-demo/` is portable. |
| Swiggy token expired (>5 days) | Delete `~/.config/voice-agent-demo/swiggy_token_<persona>.json` and re-run — it re-auths. Swiggy v1.0 has no refresh, so the 5-day token is the whole session. |
| Swiggy MCP errors mid-call | The agent catches it and falls back to a verbal suggestion. (Live-only by request; add a curated fallback list in `swiggy_mcp.py` if you want belt-and-suspenders.) |

## Files

```
examples/voice-agent-demo/
├── README.md              # this file
├── .env.example           # keys to fill in
├── .gitignore             # .env, .venv, __pycache__
├── pyproject.toml         # uv-managed deps
├── personas/
│   ├── kaustav.json
│   └── aparajita.json
└── src/voice_agent_demo/
    ├── __main__.py        # CLI entry: `uv run voice-agent`
    ├── app.py             # Textual app (Agent + Logs tabs)
    ├── agent.py           # OpenAI Agents SDK setup + tools
    ├── voice.py           # Sarvam STT + TTS + audio I/O
    ├── persona.py         # load personas/<name>.json
    ├── logs.py            # TracingProcessor → Logs tab feed
    ├── chakra_mcp.py      # ChakraMCP MCP client (CLI-token auth)
    └── swiggy_mcp.py      # Swiggy Dineout MCP client
```

## License

Same as the parent repo (MIT). The personas + script are sample data.
