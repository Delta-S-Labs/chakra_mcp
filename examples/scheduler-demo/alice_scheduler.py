"""Alice's scheduler agent — runs an inbox-serve loop and answers
`propose_slots` invocations from anyone she's granted.

Reads its config from state.json (written by setup.py). Uses the real
chakramcp Python SDK; no mocks, no shortcuts. The "calendar" is fake —
random slots between 9 AM and 5 PM in the next N days — but the
relay flow is exactly what a real agent would run.

Run in a terminal AFTER setup.py:

    python alice_scheduler.py

Stop with ctrl-c.
"""

from __future__ import annotations

import asyncio
import datetime as dt
import json
import random
import signal
import sys
from pathlib import Path

from chakramcp import AsyncChakraMCP

STATE = json.loads((Path(__file__).parent / "state.json").read_text())


def fake_propose_slots(duration_min: int, within_days: int) -> list[str]:
    """Make up four slots between 9 AM and 5 PM in the next N days.

    Replace with a real calendar lookup when wiring this to a CalDAV /
    Google Calendar / etc. The point of the demo is the relay flow,
    not the calendaring.
    """
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
    slots: list[str] = []
    for _ in range(4):
        days_out = random.randint(1, max(1, within_days))
        hour = random.randint(9, 16)
        candidate = (now + dt.timedelta(days=days_out)).replace(
            hour=hour, minute=0, second=0
        )
        slots.append(candidate.isoformat())
    slots.sort()
    return slots


async def handle(invocation: dict) -> dict:
    """One invocation = one call to propose_slots."""
    capability = invocation["capability_name"]
    inputs = invocation.get("input_preview") or {}
    print(f"  ← {capability}({inputs})")

    # Trust context — the relay verified these before delivering this
    # row. We log them so you can see what the network handed us.
    grant = invocation.get("grant_context")
    friendship = invocation.get("friendship_context")
    if grant:
        print(
            f"    grant {grant['id'][:13]}…  visibility={grant['capability_visibility']}"
            f"  granted_at={grant['granted_at']}"
        )
    if friendship:
        msg = friendship.get("proposer_message") or "—"
        print(f"    friendship {friendship['id'][:13]}…  initial message: {msg!r}")

    if capability != "propose_slots":
        return {"status": "failed", "error": f"unsupported capability: {capability}"}

    slots = fake_propose_slots(
        duration_min=int(inputs.get("duration_min", 30)),
        within_days=int(inputs.get("within_days", 7)),
    )
    print(f"  → returning {len(slots)} slots")
    return {"status": "succeeded", "output": {"slots": slots}}


async def main() -> int:
    chakra = AsyncChakraMCP(
        api_key=STATE["alice"]["api_key"],
        app_url=STATE["app_url"],
        relay_url=STATE["relay_url"],
    )

    me = await chakra.me()
    print(f"signed in as {me['user']['email']}")
    print(f"agent  : {STATE['alice']['agent_id']}")
    print()
    print("Listening for invocations… (ctrl-c to stop)")
    print()

    stop = asyncio.Event()
    loop = asyncio.get_event_loop()
    try:
        loop.add_signal_handler(signal.SIGINT, stop.set)
        loop.add_signal_handler(signal.SIGTERM, stop.set)
    except NotImplementedError:
        # Windows; ctrl-c still raises KeyboardInterrupt below.
        pass

    try:
        await chakra.inbox.serve(
            STATE["alice"]["agent_id"],
            handle,
            poll_interval_s=1.0,
            stop_event=stop,
            on_error=lambda err, inv: print(f"  ! error: {err} (inv={inv and inv.get('id')})"),
        )
    finally:
        await chakra.aclose()
    print()
    print("stopped.")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
