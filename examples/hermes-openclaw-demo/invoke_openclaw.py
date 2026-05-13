"""Hermes-as-caller: invoke OpenClaw's `suggest-recipes` capability.

OpenClaw is push-mode, so the call path is:

    invoke_openclaw.py
        ↓ (chakramcp Python SDK, authed as Hermes)
    relay POST /v1/invoke
        ↓ (forwarder mints a JWT, looks up OpenClaw's agent_card_url,
           resolves the supported_interfaces[0].url)
    POST http://127.0.0.1:18800/a2a/jsonrpc
        ↓ (mock_openclaw.py replies with a Task.completed envelope)
    relay translates A2A → ChakraMCP invocation result
        ↓
    invoke_and_wait() returns

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
from pathlib import Path

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
        help="Prompt for y/n confirmation before sending the invoke (human-in-the-loop).",
    )
    p.add_argument("--timeout", type=float, default=30.0)
    return p.parse_args()


async def main() -> int:
    args = parse_args()
    ingredients = [s.strip() for s in args.ingredients.split(",") if s.strip()]
    if not ingredients:
        print("error: --ingredients was empty after splitting", file=sys.stderr)
        return 2

    chakra = AsyncChakraMCP(
        api_key=STATE["hermes"]["api_key"],
        app_url=STATE["app_url"],
        relay_url=STATE["relay_url"],
    )

    me = await chakra.me()
    print(f"signed in as {me['user']['email']}  (Hermes side)")
    print(
        f"target     : openclaw-recipes.suggest-recipes  "
        f"(grant {STATE['grant_hermes_calls_openclaw'][:13]}…)"
    )
    print(f"ingredients: {ingredients}")
    print()

    if args.ask:
        ans = input("Send this invocation through the relay? [y/N] ").strip().lower()
        if ans not in ("y", "yes"):
            print("cancelled.")
            return 1

    try:
        result = await chakra.invoke_and_wait(
            {
                "grant_id": STATE["grant_hermes_calls_openclaw"],
                "grantee_agent_id": STATE["hermes"]["agent_id"],
                # The push-mode forwarder maps `input` straight into
                # the A2A SendMessage envelope's data part. The mock
                # accepts either {"ingredients": [...]} (this shape)
                # or a dict-of-strings.
                "input": {"ingredients": ingredients},
            },
            interval_s=0.5,
            timeout_s=args.timeout,
        )
    finally:
        await chakra.aclose()

    print(f"  status     : {result['status']}")
    print(f"  elapsed_ms : {result.get('elapsed_ms', '-')}")
    if result["status"] == "succeeded":
        recipes = (result.get("output_preview") or {}).get("recipes", [])
        source = (result.get("output_preview") or {}).get("from", "?")
        print(f"  source     : {source}")
        print(f"  recipes    : {len(recipes)}")
        for r in recipes:
            print(f"    • {r}")
        return 0
    print(f"  error      : {result.get('error_message')}")
    return 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
