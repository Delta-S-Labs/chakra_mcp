"""Bob's caller — invokes Alice's `propose_slots` through the relay.

Uses invoke_and_wait() so we can show the synchronous-feel API: send,
poll until the granter agent (Alice's scheduler in another terminal)
responds, print the result.

Run in a separate terminal AFTER setup.py and alice_scheduler.py:

    python bob_caller.py

You should see Alice's terminal log the inbox claim while this one
prints the returned slots.
"""

from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path

from chakramcp import AsyncChakraMCP

STATE = json.loads((Path(__file__).parent / "state.json").read_text())


async def main() -> int:
    chakra = AsyncChakraMCP(
        api_key=STATE["bob"]["api_key"],
        app_url=STATE["app_url"],
        relay_url=STATE["relay_url"],
    )

    me = await chakra.me()
    print(f"signed in as {me['user']['email']}")
    print(f"calling alice-scheduler.propose_slots through grant {STATE['grant_id'][:13]}…")
    print()

    try:
        result = await chakra.invoke_and_wait(
            {
                "grant_id": STATE["grant_id"],
                "grantee_agent_id": STATE["bob"]["agent_id"],
                "input": {"duration_min": 30, "within_days": 7},
            },
            interval_s=0.5,
            timeout_s=30.0,
        )
    finally:
        await chakra.aclose()

    print(f"  status     : {result['status']}")
    print(f"  elapsed_ms : {result['elapsed_ms']}")
    if result["status"] == "succeeded":
        slots = (result.get("output_preview") or {}).get("slots", [])
        print(f"  slots      : {len(slots)}")
        for slot in slots:
            print(f"    • {slot}")
        return 0
    print(f"  error      : {result.get('error_message')}")
    return 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
