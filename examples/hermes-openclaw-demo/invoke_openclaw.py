"""Hermes-as-caller: call OpenClaw's `suggest-recipes` capability.

OpenClaw is push-mode, so it takes calls only on its A2A endpoint on the
relay. `/v1/invoke`, which the SDKs' `invoke` and the CLI use, refuses a
push-mode target with 409 `chk.target.push_mode`. The call path is:

    invoke_openclaw.py
        ↓ POST /agents/<account>/openclaw-recipes/a2a/jsonrpc  (SendMessage)
          Authorization: Bearer <Hermes's API key>
          X-ChakraMCP-Caller-Agent: <Hermes's agent id>
          X-ChakraMCP-Capability: <suggest-recipes's id>
    relay: checks the friendship and the grant, mints a JWT, looks up
           OpenClaw's agent_card_url and its supported_interfaces[0].url
        ↓ POST http://127.0.0.1:18800/a2a/jsonrpc
    mock_openclaw.py replies with a completed Task
        ↓ the relay returns that reply as the response

This script demonstrates the human-in-the-loop variant: a person runs
the script (consenting to the call). For autonomous calls — say from
a scheduled job — drop `--ask` and wire the same code into your loop.

Run AFTER setup.py and (in another terminal) mock_openclaw.py:

    python invoke_openclaw.py --ingredients chicken,rice,lemon
"""

from __future__ import annotations

import argparse
import asyncio
import json
import sys
import time
import uuid
from pathlib import Path

import httpx
from chakramcp import AsyncChakraMCP

STATE = json.loads((Path(__file__).parent / "state.json").read_text())


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    p.add_argument(
        "--ingredients",
        default="chicken,rice,lemon",
        help="Comma-separated ingredient list. Default: chicken,rice,lemon",
    )
    p.add_argument(
        "--ask",
        action="store_true",
        help="Prompt for y/n confirmation before sending the call (human-in-the-loop).",
    )
    p.add_argument("--timeout", type=float, default=30.0)
    return p.parse_args()


async def main() -> int:
    args = parse_args()
    ingredients = [s.strip() for s in args.ingredients.split(",") if s.strip()]
    if not ingredients:
        print("error: --ingredients was empty after splitting", file=sys.stderr)
        return 2

    async with AsyncChakraMCP(
        api_key=STATE["hermes"]["api_key"],
        app_url=STATE["app_url"],
        relay_url=STATE["relay_url"],
    ) as chakra:
        me = await chakra.me()
    print(f"signed in as {me['user']['email']}  (Hermes side)")
    print(
        f"target     : openclaw-recipes.suggest-recipes over A2A  "
        f"(grant {STATE['grant_hermes_calls_openclaw'][:13]}…)"
    )
    print(f"ingredients: {ingredients}")
    print()

    openclaw = STATE["openclaw"]
    account = openclaw.get("account_slug")
    if not account:
        print(
            "error: state.json is from an older setup.py; run `python setup.py` again",
            file=sys.stderr,
        )
        return 2
    url = (
        f"{STATE['relay_url'].rstrip('/')}/agents/{account}/"
        f"{openclaw['agent_slug']}/a2a/jsonrpc"
    )

    if args.ask:
        ans = input("Send this call through the relay? [y/N] ").strip().lower()
        if ans not in ("y", "yes"):
            print("cancelled.")
            return 1

    request = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "SendMessage",
        "params": {
            "message": {
                "role": "user",
                "messageId": str(uuid.uuid4()),
                # OpenClaw reads the first part's data as the input.
                "parts": [{"data": {"ingredients": ingredients}}],
            }
        },
    }
    started = time.monotonic()
    async with httpx.AsyncClient(timeout=args.timeout) as http:
        res = await http.post(
            url,
            json=request,
            headers={
                "Authorization": f"Bearer {STATE['hermes']['api_key']}",
                "X-ChakraMCP-Caller-Agent": STATE["hermes"]["agent_id"],
                "X-ChakraMCP-Capability": openclaw["capability_id"],
            },
        )
    elapsed_ms = round((time.monotonic() - started) * 1000)

    try:
        reply = res.json()
    except ValueError:
        print(f"  error      : HTTP {res.status_code}: {res.text[:200]}")
        return 1
    if "error" in reply:
        err = reply["error"]
        code = (err.get("data") or {}).get("code", err.get("code"))
        print(f"  error      : {err.get('message')} ({code}, HTTP {res.status_code})")
        return 1

    task = reply.get("result") or {}
    state = (task.get("status") or {}).get("state", "?")
    print(f"  state      : {state}")
    print(f"  elapsed_ms : {elapsed_ms}")
    if state != "completed":
        print(f"  reply      : {json.dumps(task)[:200]}")
        return 1
    parts = (task.get("artifacts") or [{}])[0].get("parts") or [{}]
    output = parts[0].get("data") or {}
    recipes = output.get("recipes", [])
    print(f"  source     : {output.get('from', '?')}")
    print(f"  recipes    : {len(recipes)}")
    for r in recipes:
        print(f"    • {r}")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
