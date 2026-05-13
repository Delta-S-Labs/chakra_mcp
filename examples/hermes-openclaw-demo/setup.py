"""Provision Hermes (pull-mode) and OpenClaw (push-mode) on the relay.

Two agents on two accounts, friended both directions, with grants in
both directions. After this script:

  • Hermes is a ChakraMCP-native pull-mode agent. It runs `inbox.serve`
    (or a cron-style one-shot pull) to receive invocations.
  • OpenClaw is registered as push-mode with `agent_card_url` pointing
    at the local mock OpenClaw gateway. The relay's D2d fetcher pulls
    that card, normalizes it, and publishes it under our domain so
    callers see a consistent A2A interface.

Both register with `visibility=network` so they show up at
`/agents` (the public directory). DISCOVERY_V2 must be enabled on
the relay for that page to render.

Run order:

    python mock_openclaw.py --port 18800   # in terminal 1
    python setup.py                         # in terminal 2 (this)
    python hermes_bot.py                    # in terminal 3
    python invoke_openclaw.py               # in terminal 4 (calls openclaw)
    python invoke_hermes.py                 # in terminal 4 (calls hermes back)

Writes `state.json` next to this file so the bot/caller scripts can
load API keys + ids without re-prompting.
"""

from __future__ import annotations

import argparse
import json
import os
import secrets
import sys
import time
import urllib.error
import urllib.request

from pathlib import Path

DEFAULT_APP_URL = os.environ.get("CHAKRAMCP_APP_URL", "http://localhost:8080")
DEFAULT_RELAY_URL = os.environ.get("CHAKRAMCP_RELAY_URL", "http://localhost:8090")
DEFAULT_OPENCLAW_CARD = os.environ.get(
    "OPENCLAW_CARD_URL",
    "http://127.0.0.1:18800/.well-known/agent-card.json",
)
STATE_FILE = Path(__file__).parent / "state.json"


def http(method: str, url: str, *, token: str | None = None, body: dict | None = None) -> dict:
    headers = {"content-type": "application/json"}
    if token:
        headers["authorization"] = f"Bearer {token}"
    req = urllib.request.Request(
        url,
        method=method,
        headers=headers,
        data=json.dumps(body).encode("utf-8") if body is not None else None,
    )
    try:
        with urllib.request.urlopen(req) as resp:
            raw = resp.read().decode("utf-8")
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as err:
        body_text = err.read().decode("utf-8") if err.fp else ""
        raise RuntimeError(f"{method} {url} → {err.code}: {body_text}") from err
    except urllib.error.URLError as err:
        raise RuntimeError(
            f"{method} {url} → {err.reason}. "
            f"Is the relay running? Check CHAKRAMCP_APP_URL / CHAKRAMCP_RELAY_URL."
        ) from err


def signup(app_url: str, label: str) -> dict:
    """Create a fresh user. Returns {token, user_id, account_id, email}."""
    suffix = secrets.token_hex(4)
    email = f"demo-{label}-{int(time.time())}-{suffix}@example.com"
    print(f"  signup: {email}")
    res = http(
        "POST",
        f"{app_url}/v1/auth/signup",
        body={
            "email": email,
            "password": "demo-password-only-locally",
            "name": label.title(),
        },
    )
    return {
        "email": email,
        "token": res["token"],
        "user_id": res["user"]["id"],
        "account_id": res["memberships"][0]["account_id"],
    }


def mint_api_key(app_url: str, token: str, name: str) -> str:
    res = http("POST", f"{app_url}/v1/api-keys", token=token, body={"name": name})
    return res["plaintext"]


def register_pull_agent(
    relay_url: str, token: str, account_id: str, slug: str, name: str, description: str, tags: list[str]
) -> str:
    """Register a pull-mode agent (no agent_card_url)."""
    res = http(
        "POST",
        f"{relay_url}/v1/agents",
        token=token,
        body={
            "account_id": account_id,
            "slug": slug,
            "display_name": name,
            "description": description,
            "tags": tags,
            "visibility": "network",
        },
    )
    return res["id"]


def register_push_agent(
    relay_url: str,
    token: str,
    account_id: str,
    slug: str,
    name: str,
    description: str,
    tags: list[str],
    agent_card_url: str,
) -> str:
    """Register a push-mode agent. The relay's D2d fetcher will pull
    the canonical card from `agent_card_url` and serve a normalized
    version under our domain."""
    res = http(
        "POST",
        f"{relay_url}/v1/agents",
        token=token,
        body={
            "account_id": account_id,
            "slug": slug,
            "display_name": name,
            "description": description,
            "tags": tags,
            "visibility": "network",
            "agent_card_url": agent_card_url,
        },
    )
    return res["id"]


def add_capability(
    relay_url: str,
    token: str,
    agent_id: str,
    *,
    name: str,
    description: str,
    input_schema: dict,
    output_schema: dict,
) -> str:
    res = http(
        "POST",
        f"{relay_url}/v1/agents/{agent_id}/capabilities",
        token=token,
        body={
            "name": name,
            "description": description,
            "input_schema": input_schema,
            "output_schema": output_schema,
            "visibility": "network",
        },
    )
    return res["id"]


def propose_friendship(relay_url: str, token: str, from_agent: str, to_agent: str, msg: str) -> str:
    res = http(
        "POST",
        f"{relay_url}/v1/friendships",
        token=token,
        body={
            "proposer_agent_id": from_agent,
            "target_agent_id": to_agent,
            "proposer_message": msg,
        },
    )
    return res["id"]


def accept_friendship(relay_url: str, token: str, friendship_id: str, msg: str = "Accepted.") -> None:
    http(
        "POST",
        f"{relay_url}/v1/friendships/{friendship_id}/accept",
        token=token,
        body={"response_message": msg},
    )


def create_grant(
    relay_url: str, token: str, granter_agent: str, grantee_agent: str, capability_id: str
) -> str:
    res = http(
        "POST",
        f"{relay_url}/v1/grants",
        token=token,
        body={
            "granter_agent_id": granter_agent,
            "grantee_agent_id": grantee_agent,
            "capability_id": capability_id,
        },
    )
    return res["id"]


# ─── Capability schemas ────────────────────────────────────────────

# Hermes (pull-mode, ChakraMCP-native): a generic Q&A endpoint.
HERMES_ANSWER_QUESTION = {
    "name": "answer_question",
    "description": "Answer a free-form text question. Demo capability for hermes-bot.",
    "input_schema": {
        "type": "object",
        "required": ["question"],
        "properties": {
            "question": {"type": "string", "minLength": 1, "maxLength": 2000},
            "style": {
                "type": "string",
                "enum": ["concise", "verbose"],
                "default": "concise",
            },
        },
    },
    "output_schema": {
        "type": "object",
        "required": ["answer"],
        "properties": {
            "answer": {"type": "string"},
            "from": {"type": "string"},
        },
    },
}

# OpenClaw (push-mode, mock A2A gateway): a recipe-suggestion endpoint.
# This MUST mirror the skill the mock OpenClaw advertises in its
# Agent Card so the relay's D2d fetcher can map cap_name → A2A skill.
OPENCLAW_SUGGEST_RECIPES = {
    "name": "suggest-recipes",
    "description": "Given an ingredient list, returns 3 recipe ideas.",
    "input_schema": {
        "type": "object",
        "required": ["ingredients"],
        "properties": {
            "ingredients": {
                "type": "array",
                "items": {"type": "string"},
                "minItems": 1,
                "maxItems": 20,
            },
        },
    },
    "output_schema": {
        "type": "object",
        "required": ["recipes"],
        "properties": {
            "recipes": {"type": "array", "items": {"type": "string"}},
            "from": {"type": "string"},
        },
    },
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--app-url", default=DEFAULT_APP_URL)
    parser.add_argument("--relay-url", default=DEFAULT_RELAY_URL)
    parser.add_argument(
        "--openclaw-card-url",
        default=DEFAULT_OPENCLAW_CARD,
        help="Where the relay should fetch OpenClaw's canonical Agent Card. "
        "Default points at the local mock_openclaw.py.",
    )
    args = parser.parse_args()

    print("==> Provisioning two demo accounts")
    hermes = signup(args.app_url, "hermes")
    openclaw = signup(args.app_url, "openclaw")

    print("==> Minting API keys")
    hermes_key = mint_api_key(args.app_url, hermes["token"], "hermes-openclaw-demo")
    openclaw_key = mint_api_key(args.app_url, openclaw["token"], "hermes-openclaw-demo")

    print("==> Registering Hermes (pull-mode, ChakraMCP-native)")
    hermes_agent = register_pull_agent(
        args.relay_url,
        hermes["token"],
        hermes["account_id"],
        slug="hermes",
        name="Hermes",
        description=(
            "Demo pull-mode agent. Polls the relay inbox, answers free-form "
            "questions. Powered by the ChakraMCP Python SDK."
        ),
        tags=["demo", "qa", "pull"],
    )

    print("==> Adding answer_question capability to Hermes")
    hermes_cap = add_capability(
        args.relay_url,
        hermes["token"],
        hermes_agent,
        **HERMES_ANSWER_QUESTION,
    )

    print("==> Registering OpenClaw (push-mode → mock at " + args.openclaw_card_url + ")")
    openclaw_agent = register_push_agent(
        args.relay_url,
        openclaw["token"],
        openclaw["account_id"],
        slug="openclaw-recipes",
        name="OpenClaw Recipes",
        description=(
            "Demo push-mode agent. Backed by the mock openclaw-a2a-gateway; "
            "relay fetches its Agent Card from agent_card_url and forwards "
            "calls via JSONRPC."
        ),
        tags=["demo", "cooking", "recipes", "push", "openclaw"],
        agent_card_url=args.openclaw_card_url,
    )

    print("==> Adding suggest-recipes capability to OpenClaw")
    openclaw_cap = add_capability(
        args.relay_url,
        openclaw["token"],
        openclaw_agent,
        **OPENCLAW_SUGGEST_RECIPES,
    )

    print("==> Friendship: Hermes → OpenClaw, OpenClaw accepts")
    f1 = propose_friendship(
        args.relay_url,
        hermes["token"],
        hermes_agent,
        openclaw_agent,
        msg="Want to trade questions for recipes?",
    )
    accept_friendship(args.relay_url, openclaw["token"], f1, msg="Sure — kitchen open.")

    print("==> Grants (both directions, so either side can call the other)")
    grant_h_to_o = create_grant(
        args.relay_url, openclaw["token"], openclaw_agent, hermes_agent, openclaw_cap
    )
    print(f"   • Hermes can call OpenClaw.suggest-recipes (grant {grant_h_to_o[:13]}…)")
    grant_o_to_h = create_grant(
        args.relay_url, hermes["token"], hermes_agent, openclaw_agent, hermes_cap
    )
    print(f"   • OpenClaw can call Hermes.answer_question (grant {grant_o_to_h[:13]}…)")

    state = {
        "app_url": args.app_url,
        "relay_url": args.relay_url,
        "openclaw_card_url": args.openclaw_card_url,
        "hermes": {
            "email": hermes["email"],
            "api_key": hermes_key,
            "account_id": hermes["account_id"],
            "agent_id": hermes_agent,
            "agent_slug": "hermes",
            "capability_id": hermes_cap,
        },
        "openclaw": {
            "email": openclaw["email"],
            "api_key": openclaw_key,
            "account_id": openclaw["account_id"],
            "agent_id": openclaw_agent,
            "agent_slug": "openclaw-recipes",
            "capability_id": openclaw_cap,
        },
        "friendship_id": f1,
        # `grant_h_to_o` is held by Hermes (the grantee) — used when
        # Hermes calls OpenClaw. `grant_o_to_h` is the inverse.
        "grant_hermes_calls_openclaw": grant_h_to_o,
        "grant_openclaw_calls_hermes": grant_o_to_h,
    }
    STATE_FILE.write_text(json.dumps(state, indent=2))

    print()
    print(f"Wrote {STATE_FILE.relative_to(Path.cwd())}")
    print()
    print("Frontend check:  open http://localhost:3000/agents")
    print("                 (DISCOVERY_V2=true on the relay required)")
    print()
    print("Next:")
    print("  Terminal A (already running):  python mock_openclaw.py --port 18800")
    print("  Terminal B:                    python hermes_bot.py")
    print("  Terminal C:                    python invoke_openclaw.py")
    print("  Terminal C (later):            python invoke_hermes.py")
    return 0


if __name__ == "__main__":
    sys.exit(main())
