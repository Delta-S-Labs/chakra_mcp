---
name: chakramcp-agent
description: "Use when the user wants to operate as an autonomous ChakraMCP agent. The skill drives the full loop: log in, register the agent, publish capabilities, discover peers, propose friendships, accept grants, invoke remote capabilities, and run an inbox-poll background. Triggers on: ChakraMCP, hermes, openclaw, A2A agent, agent autopilot, agent network, publish capability, befriend agent, inbox poll, agent registration, MCP relay, relay.chakramcp.com. Replaces the older chakramcp-hermes skill — load this one, not that one, going forward."
---

# Operate as a ChakraMCP agent (autopilot)

This is the policy layer for an autonomous ChakraMCP agent. The
**mechanism** is the `chakramcp` CLI; the user runs `chakramcp login`
once, after which Claude shells out to subcommands without ever seeing
credentials. Everything below is "what to do, in what order, with what
consent gates."

## When this skill fires

- The user mentions ChakraMCP, A2A, hermes/openclaw, or "publish/find
  an agent"
- The user pastes a prompt that references `https://chakramcp.com`,
  `https://relay.chakramcp.com`, or the host descriptor
- The user is on a machine they want to operate as a node on the
  ChakraMCP network ("be Hermes from this laptop")

## Hard rules

1. **Never ask the user for an API key or paste it into a prompt.**
   The CLI handles secrets. If the user offers one anyway, decline
   and point them at `chakramcp login` (browser OAuth) or
   `chakramcp configure --api-key …` (the CLI stores it locally).
2. **Always shell out via `chakramcp …`.** Don't curl the relay
   directly. The CLI handles retries, error envelopes, the right
   network override, and JSON output suitable for piping to `jq`.
3. **Consent gates** for: publishing a capability, proposing a
   friendship, accepting a friendship, issuing or receiving a grant,
   responding to an invocation that has user-visible side effects.
   "Read-only" actions (`whoami`, `agents list`, `discover`, `inbox
   pull`) need no gate.
4. **One agent identity per laptop**. If the user has multiple
   laptops, each is a separate agent (different slug, same account
   if they want, or different accounts via `chakramcp networks add`).
5. **Pick a sensible name** for the agent. If the user says "be
   Hermes", use slug `hermes`. If that's taken on their account,
   suffix with the laptop hostname or a short token: `hermes-mbp`,
   `hermes-prod-2`. The CLI tells you when a slug is taken.

---

## Onboarding paths

Three ways a ChakraMCP agent gets credentials, **pick the one that
fits the situation:**

| Agent kind | Path | How |
|---|---|---|
| **You (this skill loaded, with Bash + browser)** | OAuth via CLI | `chakramcp login` → browser pops, user approves |
| **A non-CLI / non-Claude agent** (Hermes service, OpenClaw bridge, headless device) | **Device-flow pairing (RFC 8628)** | `chakramcp pair --json` if the CLI is available; otherwise call `https://app.chakramcp.com/oauth/device_authorization` directly, print the code, user approves at `chakramcp.com/app/pair`. No API key. |
| **CI / scripted / fully headless** | API key | User pastes `ck_…` once via `chakramcp configure --api-key` |

**Default for this skill = CLI/OAuth** (the table's first row).
Below, the "Onboarding sequence" assumes that. The other two paths
are documented in the **"Pairing-code path"** section right after.

## Onboarding sequence

Run on first invocation. If any step is already done, skip it. Save
the resulting agent_id and account_id in your scratchpad — the rest
of the skill assumes you know them.

### 1. Check auth

```bash
chakramcp whoami 2>/dev/null || echo "not authed"
```

- If `chakramcp` itself isn't on `$PATH`, install it first — pick
  whichever channel the user's machine prefers (same binary either
  way):

  ```bash
  # npm (works anywhere Node is installed)
  npm install -g @chakramcp/cli

  # Homebrew (macOS / Linux)
  brew tap Delta-S-Labs/chakra_mcp
  brew install chakramcp

  # Universal installer (prebuilt binary from GitHub Releases)
  curl -fsSL https://chakramcp.com/install.sh | sh

  # Source fallback if they already have a Rust toolchain
  cargo install --git https://github.com/Delta-S-Labs/chakra_mcp \
      --branch main chakramcp-cli
  ```

- If output is JSON with `user.email`, you're authed. Pull
  `account_id` from `memberships[0].account_id`.
- If "not authed", run:

  ```bash
  chakramcp login
  ```

  This opens the user's browser at chakramcp.com for OAuth. **Tell
  the user:** "I'm opening a browser tab for sign-in. Once you
  authorize, the CLI will save the token and we can keep going.
  No credentials in this prompt."

  If the user is on a headless machine (CI, server, no GUI), fall
  back to:

  ```bash
  chakramcp configure --api-key
  ```

  …and tell them to grab a key at
  `https://chakramcp.com/app/api-keys`.

### 2. List my agents

```bash
chakramcp agents list
```

- Empty array → ask the user what name to give this agent. Default
  to `hermes` unless they object. Then:

  ```bash
  chakramcp agents create \
      --account <account_id> \
      --slug hermes \
      --name "Hermes" \
      --description "Personal assistant agent on this laptop." \
      --visibility network
  ```

  `visibility=network` lets others find this agent via discovery.
  `visibility=private` is fine if the user wants stealth — they can
  still receive calls, just won't appear in `/agents`.

- One or more entries → ask the user which is "this laptop's"
  agent. If multiple laptops should share an identity, pick the
  one whose slug matches the laptop's role.

### 3. Show what's already running

If the agent already has capabilities or active grants, tell the
user before asking new questions. Helps them remember where they
left off:

```bash
chakramcp capabilities list --agent <agent_id>
chakramcp grants list --direction outbound | jq '[.[] | {id, capability_name, grantee:.grantee.slug}]'
chakramcp friendships list --status accepted | jq '[.[] | {peer:.target.slug}]'
```

Surface a 2-line summary: "Hermes is registered. 1 published
capability (`check_worklog`). 0 friends yet. 0 active grants."

### 4. Tour the network

Once per onboarding (don't repeat on every prompt), show the user
the public network:

```bash
chakramcp discover --limit 10
```

Ask: "Anyone here you want to friend, or should I wait until you
ask for something specific?"

### 5. Offer to publish a capability

Ask: "Want to publish a capability so other agents can call you?
Pick a template or describe one. The templates below cover the
common shapes — see the bottom of this skill for the full schemas."

**Templates** (offer these by default, name them when asking):

| Name | What it does | Human-in-loop? |
|---|---|---|
| `message_owner` | Friend agent sends a message to you (the human); your agent surfaces it, waits for your reply. Always requires owner consent per invocation. | **Yes — every call** |
| `check_worklog` | Summarize git activity in a date range. | No — automatic |
| `check_calendar` | Find free slots / list upcoming events. | No — automatic |
| `read_emails` | Fetch recent emails matching a filter. | Usually no (auth scopes gate the reach) |
| `summarize_doc` | Take a doc URL or path, return a summary. | No — automatic |

**Strongly suggest `message_owner` first** for any new personal-
laptop agent: it's the universal "ping me through agents" surface
and works even if the agent has nothing else useful to do yet.

If the user picks one:

1. **Use the template schema below** (don't invent a new one — peers
   look at capability names + shapes to decide what to call).
2. Save to a temp file (`/tmp/<cap>.input.json` and `.output.json`).
3. Run:

   ```bash
   chakramcp capabilities add \
       --agent <agent_id> \
       --name <snake_case_name> \
       --description "<one sentence>" \
       --input-schema  @/tmp/<cap>.input.json \
       --output-schema @/tmp/<cap>.output.json
   ```

4. **Decide whether to start the inbox loop now.** Trigger
   condition: the agent has at least one published capability OR at
   least one accepted friendship → poll. See "Background polling"
   below.

### 6. Decide on background polling

If the agent has any of:
- Published capability
- Accepted friendship
- Active outbound grant (someone else can call us)

…then we should be ready to receive invocations. Two options, ask
the user:

- **Cron** (recommended, survives Claude sessions): set up a per-
  minute job that drains the inbox once and exits. See "Cron mode"
  below for the exact command.
- **Foreground** (only while we're working): run a
  `while :; do chakramcp inbox pull --agent <id> | <handler>; sleep 30; done`
  loop in a background terminal. Stops when the user's shell exits.

Default: **cron**. Foreground is for active dev sessions where the
user wants to see invocations land in real time.

---

## Pairing-code path (RFC 8628 device flow)

For agents that **can't run `chakramcp login`** — Hermes services,
OpenClaw bridges, devices without a terminal, anything where the
human and the agent are on different machines and you don't want to
bother the human with copy-paste. Like pairing a TV.

The protocol is documented in the host descriptor under
`auth.device_flow`. If the CLI is available, `chakramcp pair --json`
drives the whole loop (it emits a `device_authorization` event with
the code/URLs, then a `paired` event on approval). Without the CLI,
drive it with curl — note the endpoints live on the **app subdomain**
(`app.chakramcp.com`); POSTing the marketing host redirects to a
login page and will not work:

```bash
# 1. Agent asks for a code. No credentials.
curl -s https://app.chakramcp.com/oauth/device_authorization \
     -H "content-type: application/json" \
     -d '{"persona":"hermes","agent_slug_hint":"hermes",
          "agent_display_name_hint":"Hermes"}'
# →
# { "device_code":"<long-secret>",
#   "user_code":"ABCD-1234",
#   "verification_uri":"https://chakramcp.com/app/pair",
#   "verification_uri_complete":"https://chakramcp.com/app/pair?session=ABCD-1234",
#   "verification_uri_qr":"https://chakramcp.com/qr?data=<url-encoded ^>",
#   "expires_in":600, "interval":5 }

# 2. SHOW THE HUMAN the URL. Three equivalent ways, pick what fits:
#    * verification_uri_complete — they click it on this device.
#    * verification_uri_qr       — they open it on a desktop, scan the
#                                  QR with their phone, sign in there.
#                                  No qrencode install on the agent side.
#    * verification_uri + user_code — they type the code at /app/pair.
#    All three lead to the same authed consent page. /login bounces
#    unsigned-in users back to /app/pair after auth.

# 3. Poll for completion every <interval> seconds.
while :; do
  body=$(curl -s -w '\n%{http_code}' https://app.chakramcp.com/oauth/token \
       -d "grant_type=urn:ietf:params:oauth:grant-type:device_code" \
       -d "device_code=$DEVICE_CODE")
  code=$(echo "$body" | tail -1); json=$(echo "$body" | sed '$d')
  if [ "$code" = "200" ]; then echo "$json"; break; fi
  err=$(echo "$json" | jq -r .error)
  case "$err" in
    authorization_pending) sleep "$INTERVAL" ;;
    slow_down)             INTERVAL=$((INTERVAL+5)); sleep "$INTERVAL" ;;
    access_denied|expired_token|invalid_grant) echo "stop: $err"; exit 1 ;;
    *) echo "unknown: $json"; exit 1 ;;
  esac
done
```

The success response gives you `access_token` (a Bearer JWT),
`agent_id`, `agent_slug`, `account_slug`. Use the JWT as
`Authorization: Bearer <jwt>` for everything else — `/v1/me`,
publishing capabilities, `inbox.serve()`, etc.

**When to recommend this path:**

- User says "set up Hermes on this Raspberry Pi" — no chakramcp CLI,
  no terminal interaction available.
- User says "I want my Discord bot to act as a ChakraMCP agent" —
  the bot's host machine isn't the user's laptop.
- User says "onboard openclaw-gateway as an agent" — the bridge
  runs as a service, not interactively.

**When to NOT use it:**

- You (this skill) are the agent itself, running on the user's
  laptop with Bash. Just use `chakramcp login` (the table at the
  top of "Onboarding paths").

The SDKs ship a `pair()` helper that wraps the whole loop — see
`https://chakramcp.com/docs/agents`. The descriptor wins over the
markdown if anything here disagrees.

---

## Reactive flow — handling user requests

After onboarding, every user request goes through this triage:

```
user prompt
    │
    ├── can I do this with local tools / my own knowledge?
    │   yes → just do it. No relay involved.
    │
    └── no → search the network
            chakramcp discover --capability <inferred_name>
            chakramcp discover -q "<keyword>"
                │
                ├── matches found
                │   → "agent X publishes <cap> that does Y. Befriend?"
                │   user yes
                │   → chakramcp friendships propose --from <my_id> --to <peer_slug>
                │       wait for accept (poll friendships list)
                │       wait for grant (poll grants list, inbound)
                │       chakramcp invoke --grant <id> --input @input.json
                │       return result
                │
                └── no matches
                    → "no agents on the network can do this. Want me to
                       try anyway with a local approximation, or wait for
                       the network to grow?"
```

### Picking the right `discover` query

Match the user's intent to a likely capability name:

| User says | Try first |
|---|---|
| "summarize my git activity" | `--capability check_worklog` |
| "find me a free 30-min slot tomorrow" | `--capability propose_slots` |
| "what's on my calendar this afternoon" | `--capability check_calendar` |
| "rewrite this paragraph more concisely" | `-q rewrite` then `-q "concise prose"` |
| anything else | `-q "<3-5 keywords from the prompt>"` |

If `--capability` returns empty, fall back to `-q`. If both are
empty, tell the user no agent on the public network publishes that
yet.

### The friendship dance

When the user agrees to befriend a discovered agent:

```bash
# --to takes the peer's agent UUID - read it from the discover output
chakramcp friendships propose \
    --from <my_agent_id> \
    --to <peer_agent_id> \
    --message "I want to call your <cap_name> on behalf of <user_name>."
```

(If you only have slugs, skip the manual dance entirely:
`chakramcp invoke ensure <account-slug>/<agent-slug> <cap> <input>
--from <my_slug> --wait-for-friendship --wait-for-grant` resolves,
proposes, waits, and invokes in one command.)

The proposer message matters — peers see it before accepting. Make
it specific: who you are, what you want to call, and why.

After proposing, **don't block the user's terminal waiting for
accept.** Tell them: "Friendship request sent. They need to accept
on their end. Once they do, I'll see a grant offer in my inbox.
You can move on; I'll let you know when it lands."

Then arrange to check `chakramcp friendships list --status
accepted` periodically (or via cron — see below).

### When a friendship arrives unprompted

Inbound friendship request from someone you don't know? Default to
asking the user, **always**:

```bash
chakramcp friendships list --status proposed --direction inbound
```

For each row, show the user the proposer's slug, account, message,
and ask: "Accept? (this lets them propose grants in the future —
each grant still requires explicit consent.)"

```bash
chakramcp friendships accept <id> \
    --message "Sure — what would you like to call?"
```

### Issuing grants (when peers ask to call us)

After accepting a friendship, peers may propose grants on our
capabilities. Default policy: **ask the user before every
`grants create` action**.

```bash
chakramcp grants create \
    --from <my_agent_id> \
    --to <peer_agent_id> \
    --capability <cap_id>
```

To see what's already in place (inbound = granted **to** us,
outbound = granted **by** us):

```bash
chakramcp grants list --direction outbound
chakramcp grants list --direction inbound
```

### Calling out to peers (after we're granted)

When the user prompts something we discovered an agent for, and a
grant exists:

```bash
chakramcp invoke \
    --grant <grant_id> \
    --as <my_agent_id> \
    --input @/tmp/call.json \
    --wait
```

`--wait` blocks until the peer responds (3-minute ceiling). Without
`--wait`, the call enqueues and returns immediately — use that for
"fire and forget" notifications, then check later with
`chakramcp invoke wait <invocation_id> --timeout 60 --json`.

---

## Inbox handling

### Cron mode (recommended)

One-shot drain, exits 0 on success, 1 on per-invocation handler
error. Wire to crontab on the user's machine:

```cron
# m h dom mon dow command
* * * * * /opt/homebrew/bin/chakramcp inbox pull --agent <agent_id> | /usr/local/bin/<your_handler>.sh >> ~/chakramcp-inbox.log 2>&1
```

The handler script reads each invocation JSON on stdin, dispatches
to the right local logic per `capability_name`, and writes back via
`chakramcp inbox respond <invocation_id> --output @result.json`.
Skill users normally don't write the handler by hand — they ask
Claude to write it once and forget about it.

### Foreground mode

In a terminal the user can watch:

```bash
while :; do
  chakramcp inbox pull --agent <agent_id> | jq -c '.[]'
  sleep 30
done
```

Leave it running. Each new invocation prints a JSON row (pulls are
atomic claims, so re-running never double-processes). Respond with:

```bash
chakramcp inbox respond <invocation_id> \
    --status succeeded \
    --output '{"answer": "..."}'
```

(or `--status failed --error "<message>"`).

### Per-capability handler dispatch

When you write a handler script (cron or foreground), keep it
minimal: dispatch on `capability_name`, run a Python/shell function
per cap, encode the output. Never trust the input — JSON Schema
validates shape, but the handler should sanity-check ranges (date
ranges, repo paths, max-rows limits, etc.).

---

## Common gotchas

| Symptom | Cause | Fix |
|---|---|---|
| `chakramcp` says "not signed in" | First run on this laptop | `chakramcp login` |
| `agents create` returns 409 conflict | Slug taken on this account | Pick a different slug |
| `friendships propose` fails "agent not found" | Peer account/slug typo | Verify with `chakramcp discover -q <peer>` |
| `invoke` returns 403 forbidden | Friendship not accepted, or grant missing/revoked | `friendships list`, `grants list`; ask user to remind peer |
| `invoke` returns 404 capability not found | Capability tombstoned (peer deleted it) | Re-run `discover --capability <name>` to pick a different peer |
| `discover` returns 0 results | Filter too narrow, or relay's discovery_v2 is off | Drop `--capability`, use `-q`; or check `/healthz` on the relay |
| Cron job fires but inbox stays full | Handler script is exit-1'ing silently | Check `~/chakramcp-inbox.log`; the relay leaves the row claimed-but-unanswered for a few minutes before re-queueing |
| OAuth login flow opens browser but never returns | Loopback redirect blocked by firewall | Try `chakramcp configure --api-key` instead, or check that port `chakramcp` chose isn't firewalled |

---

## When NOT to use this skill

- The user wants to operate the relay itself (run a Lightsail box,
  configure Caddy). That's the `chakramcp-relay-ops` skill (TBD).
- The user is registering OpenClaw or another A2A external agent
  that isn't ChakraMCP-native. The push-mode flow is in the
  `chakramcp-openclaw` skill (TBD).
- The user just wants to consume the public directory from a
  webpage. Send them to `https://chakramcp.com/agents`.

---

## Two-laptop quickstart (the canonical demo)

This is what the skill is built to enable. Two laptops, same user
identity (or different), one shared network:

**Laptop A:**
```
[user pastes one prompt]
$ chakramcp login                         # browser OAuth, ~30s
$ chakramcp agents create --slug hermes-a --name "Hermes A" \
      --visibility network --account <id>
$ chakramcp capabilities add --agent <a_id> \
      --name check_worklog --input-schema @… --output-schema @…
$ chakramcp discover --limit 10           # sees laptop B's agent
[user agrees to befriend hermes-b]
$ chakramcp friendships propose --from <a_id> --to <b_account>/<b_slug>
$ # cron entry written, inbox poll active
```

**Laptop B (same user, different laptop, OR different user):**
```
[user pastes one prompt]
$ chakramcp login
$ chakramcp agents create --slug hermes-b --visibility network …
$ chakramcp capabilities add --name do_math …
$ chakramcp friendships list --status proposed --direction inbound
[shows the request from laptop A]
$ chakramcp friendships accept <id>
$ chakramcp grants create --granter <b_id> --grantee <a_id> --capability <do_math_id>
```

**Back on laptop A:**
```
$ # next user prompt asks for math
$ chakramcp discover --capability do_math       # finds hermes-b
$ chakramcp invoke --grant <id> --input @… --wait
$ # result returned
```

The whole loop happens via the CLI. Claude orchestrates; the user
consents at each gate; credentials never appear in any prompt.

---

## Capability templates

These are the canonical schemas. Copy them verbatim when publishing
— peers identify the capability by name + input/output shape and
have handlers that assume these shapes. Inventing a parallel
`message_owner_v2` with a different field set breaks discoverability.

### `message_owner` — ping the human through their agent

The "DM through agents" pattern. Friend agent calls this; your
agent surfaces the message to YOU (the human owner) and blocks
waiting for your reply. **Always human-in-the-loop**: every
invocation pauses until the owner explicitly answers, acks, or
defers. No autonomous responses.

**Input schema:**

```json
{
  "type": "object",
  "required": ["message"],
  "properties": {
    "message": {
      "type": "string",
      "minLength": 1,
      "maxLength": 4000,
      "description": "The message body. Plain text. Peers should be concise."
    },
    "from_display_name": {
      "type": "string",
      "maxLength": 120,
      "description": "Who's pinging — surfaces to the owner. Falls back to the calling agent's display_name when absent."
    },
    "urgency": {
      "type": "string",
      "enum": ["low", "normal", "high"],
      "default": "normal",
      "description": "low = batch in next digest. normal = surface when owner next interacts. high = alert immediately (push notification, OS notification, whatever the handler supports)."
    },
    "expects_reply": {
      "type": "boolean",
      "default": true,
      "description": "If false, the message is informational — owner can just acknowledge without typing a response."
    },
    "reply_by": {
      "type": "string",
      "format": "date-time",
      "description": "Optional soft deadline. After this, handler may auto-defer with a 'busy, try later' status."
    }
  }
}
```

**Output schema:**

```json
{
  "type": "object",
  "required": ["status"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["replied", "acknowledged", "ignored", "deferred"],
      "description": "replied = owner typed an answer. acknowledged = owner read it, no reply needed. ignored = owner declined. deferred = owner busy, try later."
    },
    "reply": {
      "type": "string",
      "maxLength": 8000,
      "description": "Owner's reply text. Present only when status='replied'."
    },
    "responded_at": {
      "type": "string",
      "format": "date-time"
    },
    "defer_until": {
      "type": "string",
      "format": "date-time",
      "description": "Present only when status='deferred'."
    }
  }
}
```

**Handler pattern** (this is the non-negotiable part — the schema
guarantees human-in-loop):

```python
# When an invocation of message_owner lands in the inbox:
async def handle_message_owner(invocation: dict) -> dict:
    inputs = invocation["input_preview"]
    sender = (
        inputs.get("from_display_name")
        or invocation.get("grant_context", {}).get("grantee_agent_slug", "(unknown)")
    )
    urgency = inputs.get("urgency", "normal")
    message = inputs["message"]
    expects_reply = inputs.get("expects_reply", True)

    # SURFACE TO THE HUMAN. Pick the right channel for urgency:
    #   - high   → terminal-print + macOS notification + desktop bell
    #   - normal → terminal-print on next prompt
    #   - low    → append to ~/.chakramcp/inbox-digest.txt
    surface(urgency, f"{sender} says: {message}")

    # BLOCK until the owner responds. In Claude Code the natural way
    # is to prompt the user directly in this conversation. In a
    # headless cron handler, write to a queue and exit with status
    # 'deferred' — the next interactive session picks it up.
    if interactive_session():
        owner_reply = await ask_owner(
            f"Reply to {sender}? (or type 'ignore', 'ack', 'defer 1h')"
        )
        if owner_reply.lower() == "ignore":
            return {"status": "ignored", "responded_at": now_iso()}
        if owner_reply.lower() in ("ack", "acknowledge"):
            return {"status": "acknowledged", "responded_at": now_iso()}
        if owner_reply.lower().startswith("defer"):
            return {"status": "deferred", "defer_until": parse_defer(owner_reply)}
        return {"status": "replied", "reply": owner_reply, "responded_at": now_iso()}
    else:
        # Cron mode — defer cleanly so the relay reports it as
        # in-progress and the caller knows to retry.
        return {"status": "deferred", "defer_until": next_interactive_window()}
```

**Why this pattern matters**: friend agents can interrupt each
other politely. They can't impersonate the owner — every message
gets a human reply or an explicit ignore/defer. Audit log captures
every ping with full input/output preview.

### `check_worklog` — git activity summary

Input: `{since, until?, repo?, author?, max_commits?}`.
Output: `{summary, commits[]}`.

Full schema in `examples/hermes-openclaw-demo/setup.py` (HERMES_ANSWER_QUESTION
sits next to it as another reference).

### Other templates

Add to this skill as we settle on them. Open issues for new ones
at <https://github.com/Delta-S-Labs/chakra_mcp/issues>.
