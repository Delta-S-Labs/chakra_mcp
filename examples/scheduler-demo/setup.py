"""Provision two ChakraMCP accounts that can talk to each other.

Creates Alice and Bob (fresh emails per run, so the demo is reproducible),
mints API keys, registers an agent for each, exposes Alice's
`propose_slots` capability, friends them, and grants Bob's agent access
to the capability. Writes the result to `state.json` next to this file.

Run once:

    python setup.py
    # → state.json appears with API keys, agent ids, grant id

Then run the two agents in separate terminals (see README).
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
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as err:
        body_text = err.read().decode("utf-8") if err.fp else ""
        raise RuntimeError(f"{method} {url} → {err.code}: {body_text}") from err


def signup(app_url: str, label: str) -> dict:
    """Create a fresh user. Returns {token, user, account_id}."""
    suffix = secrets.token_hex(4)
    email = f"demo-{label}-{int(time.time())}-{suffix}@example.com"
    print(f"  signup: {email}")
    res = http(
        "POST",
        f"{app_url}/v1/auth/signup",
        body={"email": email, "password": "demo-password-only-locally", "name": label.title()},
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


def register_agent(relay_url: str, token: str, account_id: str, slug: str, name: str) -> str:
    res = http(
        "POST",
        f"{relay_url}/v1/agents",
        token=token,
        body={
            "account_id": account_id,
            "slug": slug,
            "display_name": name,
            "description": "Demo scheduler agent — see examples/scheduler-demo/.",
            "visibility": "network",
        },
    )
    return res["id"]


def add_capability(relay_url: str, token: str, agent_id: str) -> str:
    body = {
        "name": "propose_slots",
        "description": "Return a list of available 30-minute slots in the next N days.",
        "input_schema": {
            "type": "object",
            "required": ["duration_min", "within_days"],
            "properties": {
                "duration_min": {"type": "integer", "minimum": 5, "maximum": 240},
                "within_days": {"type": "integer", "minimum": 1, "maximum": 30},
            },
        },
        "output_schema": {
            "type": "object",
            "required": ["slots"],
            "properties": {
                "slots": {
                    "type": "array",
                    "items": {"type": "string", "format": "date-time"},
                }
            },
        },
        "visibility": "network",
    }
    res = http(
        "POST",
        f"{relay_url}/v1/agents/{agent_id}/capabilities",
        token=token,
        body=body,
    )
    return res["id"]


def propose_friendship(relay_url: str, token: str, from_agent: str, to_agent: str) -> str:
    res = http(
        "POST",
        f"{relay_url}/v1/friendships",
        token=token,
        body={
            "proposer_agent_id": from_agent,
            "target_agent_id": to_agent,
            "proposer_message": "Want to schedule a meeting through the network?",
        },
    )
    return res["id"]


def accept_friendship(relay_url: str, token: str, friendship_id: str) -> None:
    http(
        "POST",
        f"{relay_url}/v1/friendships/{friendship_id}/accept",
        token=token,
        body={"response_message": "Sure thing."},
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--app-url", default=DEFAULT_APP_URL)
    parser.add_argument("--relay-url", default=DEFAULT_RELAY_URL)
    args = parser.parse_args()

    print("==> Provisioning two demo accounts")
    alice = signup(args.app_url, "alice")
    bob = signup(args.app_url, "bob")

    print("==> Minting API keys (the SDK auths with these)")
    alice_key = mint_api_key(args.app_url, alice["token"], "scheduler-demo")
    bob_key = mint_api_key(args.app_url, bob["token"], "scheduler-demo")

    print("==> Registering Alice's scheduler agent")
    alice_agent = register_agent(
        args.relay_url, alice["token"], alice["account_id"], "alice-scheduler", "Alice Scheduler"
    )

    print("==> Adding propose_slots capability")
    capability = add_capability(args.relay_url, alice["token"], alice_agent)

    print("==> Registering Bob's caller agent")
    bob_agent = register_agent(
        args.relay_url, bob["token"], bob["account_id"], "bob-caller", "Bob Caller"
    )

    print("==> Friendship: Alice → Bob, Bob accepts")
    friendship = propose_friendship(args.relay_url, alice["token"], alice_agent, bob_agent)
    accept_friendship(args.relay_url, bob["token"], friendship)

    print("==> Grant: Alice gives Bob's agent access to propose_slots")
    grant = create_grant(args.relay_url, alice["token"], alice_agent, bob_agent, capability)

    state = {
        "app_url": args.app_url,
        "relay_url": args.relay_url,
        "alice": {
            "email": alice["email"],
            "api_key": alice_key,
            "agent_id": alice_agent,
            "capability_id": capability,
        },
        "bob": {
            "email": bob["email"],
            "api_key": bob_key,
            "agent_id": bob_agent,
        },
        "friendship_id": friendship,
        "grant_id": grant,
    }
    STATE_FILE.write_text(json.dumps(state, indent=2))
    print()
    print(f"Wrote {STATE_FILE.relative_to(Path.cwd())}")
    print()
    print("Next:")
    print("  Terminal A:  python alice_scheduler.py")
    print("  Terminal B:  python bob_caller.py")
    return 0


if __name__ == "__main__":
    sys.exit(main())
