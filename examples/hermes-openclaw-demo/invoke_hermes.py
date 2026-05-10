"""OpenClaw-as-caller: invoke Hermes's `answer_question` capability.

The reverse direction. OpenClaw was issued a grant to call Hermes
during setup. Since Hermes is pull-mode, the call path is:

    invoke_hermes.py  (authed as OpenClaw via API key)
        ↓
    relay POST /v1/invoke   →  enqueued in Hermes's inbox
        ↓
    hermes_bot.py picks it up via inbox.serve / pull
        ↓
    hermes_bot.py POSTs result back via /v1/invocations/<id>/result
        ↓
    invoke_and_wait() returns the final state

In production OpenClaw is a separate runtime (not Python). For the
demo we use the OpenClaw account's API key from the same Python
process — it's all the relay sees anyway.

Hermes must be running (`python hermes_bot.py`) — this script will
hang otherwise until --timeout expires.

Run:

    python invoke_hermes.py --question "what is openclaw?"
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
        "--question",
        default="what is chakramcp?",
        help='Free-form question for Hermes. Default: "what is chakramcp?"',
    )
    p.add_argument(
        "--style",
        choices=("concise", "verbose"),
        default="concise",
    )
    p.add_argument("--timeout", type=float, default=30.0)
    return p.parse_args()


async def main() -> int:
    args = parse_args()

    chakra = AsyncChakraMCP(
        api_key=STATE["openclaw"]["api_key"],
        app_url=STATE["app_url"],
        relay_url=STATE["relay_url"],
    )

    me = await chakra.me()
    print(f"signed in as {me['user']['email']}  (OpenClaw side)")
    print(
        f"target     : hermes.answer_question  "
        f"(grant {STATE['grant_openclaw_calls_hermes'][:13]}…)"
    )
    print(f"question   : {args.question!r}")
    print()

    try:
        result = await chakra.invoke_and_wait(
            {
                "grant_id": STATE["grant_openclaw_calls_hermes"],
                "grantee_agent_id": STATE["openclaw"]["agent_id"],
                "input": {"question": args.question, "style": args.style},
            },
            interval_s=0.5,
            timeout_s=args.timeout,
        )
    finally:
        await chakra.aclose()

    print(f"  status     : {result['status']}")
    print(f"  elapsed_ms : {result.get('elapsed_ms', '-')}")
    if result["status"] == "succeeded":
        out = result.get("output_preview") or {}
        print(f"  source     : {out.get('from', '?')}")
        print(f"  answer     : {out.get('answer', '')}")
        return 0
    print(f"  error      : {result.get('error_message')}")
    return 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
