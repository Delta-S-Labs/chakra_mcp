#!/usr/bin/env python3
"""Reference pull-mode worker — autonomous capability only.

The smallest possible long-running ChakraMCP agent. It registers (or
reuses) an agent, publishes a `propose_slots` capability with
``semantics="autonomous"`` set explicitly, and runs ``inbox.serve()``
forever. Every invocation is handled by the SDK's autonomous path —
the handler returns a result dict and the SDK posts it back to the
relay.

Mirrors the shape of ``examples/scheduler-demo/alice_scheduler.py``
but stripped to the bare reference: env-var config, no state.json,
no setup script. Copy this file as the starting point for any
autonomous worker.

Configuration (env vars):
    CHAKRAMCP_API_KEY      required — `ck_…` from `chakramcp keys create`
    CHAKRAMCP_AGENT_ID     required — the agent that will serve the inbox
    CHAKRAMCP_APP_URL      optional — defaults to https://chakramcp.com
    CHAKRAMCP_RELAY_URL    optional — defaults to https://relay.chakramcp.com
    CHAKRAMCP_PUBLISH      optional — set to "1" to (re-)publish the
                           `propose_slots` capability on startup. Safe
                           to leave unset on subsequent runs.

Run:
    export CHAKRAMCP_API_KEY=ck_…
    export CHAKRAMCP_AGENT_ID=01HXXXXXXXXXXXXXXXXXXXXXXX
    python pull_worker_autonomous.py

Stop with ctrl-c.
"""

from __future__ import annotations

import asyncio
import datetime as dt
import os
import random
import signal
import sys

from chakramcp import AsyncChakraMCP, ChakraMCPError


PROPOSE_SLOTS_CAPABILITY = {
    "name": "propose_slots",
    "description": "Suggest up to four meeting slots in the next N days.",
    # Explicit even though `autonomous` is the relay-side default — making
    # the choice visible in the source is the whole point of the
    # reference (see issue #69, PR 1 / PR 2).
    "semantics": "autonomous",
    "input_schema": {
        "type": "object",
        "properties": {
            "duration_min": {"type": "integer", "minimum": 5, "maximum": 480, "default": 30},
            "within_days": {"type": "integer", "minimum": 1, "maximum": 60, "default": 7},
        },
    },
    "output_schema": {
        "type": "object",
        "required": ["slots"],
        "properties": {
            "slots": {"type": "array", "items": {"type": "string", "format": "date-time"}},
        },
    },
    "visibility": "network",
}


def fake_propose_slots(duration_min: int, within_days: int) -> list[str]:
    """Toy calendar — swap for a real CalDAV / Google Calendar lookup."""
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
    """Single autonomous handler. Receives an Invocation, returns a result."""
    capability = invocation.get("capability_name")
    inputs = invocation.get("input_preview") or {}
    print(f"  ← {capability}({inputs})", flush=True)

    if capability != "propose_slots":
        return {"status": "failed", "error": f"unsupported capability: {capability}"}

    slots = fake_propose_slots(
        duration_min=int(inputs.get("duration_min", 30)),
        within_days=int(inputs.get("within_days", 7)),
    )
    print(f"  → returning {len(slots)} slots", flush=True)
    return {"status": "succeeded", "output": {"slots": slots}}


async def ensure_capability(chakra: AsyncChakraMCP, agent_id: str) -> None:
    """(Re-)publish `propose_slots` when CHAKRAMCP_PUBLISH=1.

    Skipped by default so repeat runs don't 409 on the unique
    (agent, name) index. Real workers usually publish once at deploy
    time, not every cold start.
    """
    if os.environ.get("CHAKRAMCP_PUBLISH") != "1":
        return
    try:
        await chakra.agents.capabilities.create(agent_id, PROPOSE_SLOTS_CAPABILITY)
        print(f"  published capability: {PROPOSE_SLOTS_CAPABILITY['name']}")
    except ChakraMCPError as err:
        # Most common case: a capability with this name already exists.
        # The relay returns conflict; we treat that as success.
        print(f"  capability publish skipped: {err}", file=sys.stderr)


async def main() -> int:
    api_key = os.environ.get("CHAKRAMCP_API_KEY")
    agent_id = os.environ.get("CHAKRAMCP_AGENT_ID")
    if not api_key or not agent_id:
        print(
            "error: CHAKRAMCP_API_KEY and CHAKRAMCP_AGENT_ID must be set.",
            file=sys.stderr,
        )
        return 2

    chakra = AsyncChakraMCP(
        api_key=api_key,
        app_url=os.environ.get("CHAKRAMCP_APP_URL", "https://chakramcp.com"),
        relay_url=os.environ.get("CHAKRAMCP_RELAY_URL", "https://relay.chakramcp.com"),
    )

    me = await chakra.me()
    print(f"signed in as {me['user']['email']}")
    print(f"agent  : {agent_id}")
    await ensure_capability(chakra, agent_id)
    print()
    print("Listening for invocations… (ctrl-c to stop)")
    print()

    stop = asyncio.Event()
    loop = asyncio.get_event_loop()
    try:
        loop.add_signal_handler(signal.SIGINT, stop.set)
        loop.add_signal_handler(signal.SIGTERM, stop.set)
    except NotImplementedError:
        # Windows; KeyboardInterrupt still propagates.
        pass

    try:
        await chakra.inbox.serve(
            agent_id,
            handle,
            poll_interval_s=2.0,
            stop_event=stop,
            on_error=lambda err, inv: print(
                f"  ! error: {err} (inv={inv and inv.get('id')})",
                file=sys.stderr,
            ),
        )
    finally:
        await chakra.aclose()
    print()
    print("stopped.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(asyncio.run(main()))
    except KeyboardInterrupt:
        # asyncio cancellation may surface as KeyboardInterrupt on some
        # platforms; swallow it for a clean exit.
        sys.exit(0)
