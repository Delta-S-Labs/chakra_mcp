#!/usr/bin/env python3
"""Reference pull-mode worker — autonomous + human-in-the-loop.

This worker handles two capabilities in one ``inbox.serve()`` loop:

* ``propose_slots`` — autonomous. The SDK runs ``handle()`` and posts
  the returned result back to the relay.
* ``message_owner`` — human-in-the-loop. The SDK routes the invocation
  to ``human_handler()`` for **side-effects only**; it does NOT post a
  result. The row stays ``in_progress`` until a human resolves it
  out-of-band via::

      chakramcp message reply <invocation_id> "<reply_text>"

  which posts the wire result with ``confirmed_by_human: true`` and
  satisfies the relay's HITL gate (PR 2 of issue #69).

Our ``human_handler()`` implementation writes each pending invocation
to ``./pending/<invocation_id>.json`` and prints a one-line summary to
stderr. A human operator (or a higher-level UI watching the directory)
reads those files and replies via the CLI.

Bonus: this worker also demonstrates the **outbound** side. After
starting up it can fire a single ``message_owner`` ping to a peer
using cli-v0.1.2's ``chakramcp invoke ensure`` (issue #68) — the same
sugar a real worker would use to autonomously message a peer without
hand-rolling the friendship + grant + invoke dance. Enable it with
``CHAKRAMCP_PING_PEER=<peer-slug>``.

Configuration (env vars):
    CHAKRAMCP_API_KEY      required — `ck_…` from `chakramcp keys create`
    CHAKRAMCP_AGENT_ID     required — the agent that will serve the inbox
    CHAKRAMCP_AGENT_SLUG   required when CHAKRAMCP_PING_PEER is set;
                           passed to `chakramcp invoke ensure --from`.
    CHAKRAMCP_APP_URL      optional — defaults to https://chakramcp.com
    CHAKRAMCP_RELAY_URL    optional — defaults to https://relay.chakramcp.com
    CHAKRAMCP_PUBLISH      optional — set to "1" to (re-)publish both
                           capabilities on startup.
    CHAKRAMCP_PENDING_DIR  optional — defaults to "./pending".
    CHAKRAMCP_PING_PEER    optional — `<account-slug>/<agent-slug>` to
                           ping via `chakramcp invoke ensure` at startup.
    CHAKRAMCP_PING_TEXT    optional — message body for the ping;
                           defaults to "hello from the reference worker".

Run:
    export CHAKRAMCP_API_KEY=ck_…
    export CHAKRAMCP_AGENT_ID=01HXXXXXXXXXXXXXXXXXXXXXXX
    python pull_worker_with_hitl.py

Stop with ctrl-c.
"""

from __future__ import annotations

import asyncio
import datetime as dt
import json
import os
import random
import signal
import subprocess
import sys
from pathlib import Path

from chakramcp import AsyncChakraMCP, ChakraMCPError, get_template


PROPOSE_SLOTS_CAPABILITY = {
    "name": "propose_slots",
    "description": "Suggest up to four meeting slots in the next N days.",
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


def pending_dir() -> Path:
    path = Path(os.environ.get("CHAKRAMCP_PENDING_DIR", "./pending"))
    path.mkdir(parents=True, exist_ok=True)
    return path


def fake_propose_slots(duration_min: int, within_days: int) -> list[str]:
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
    """Autonomous path. Called only for autonomous capabilities."""
    capability = invocation.get("capability_name")
    inputs = invocation.get("input_preview") or {}
    print(f"  ← {capability}({inputs})", flush=True)

    if capability == "propose_slots":
        slots = fake_propose_slots(
            duration_min=int(inputs.get("duration_min", 30)),
            within_days=int(inputs.get("within_days", 7)),
        )
        print(f"  → returning {len(slots)} slots", flush=True)
        return {"status": "succeeded", "output": {"slots": slots}}

    return {"status": "failed", "error": f"unsupported capability: {capability}"}


async def human_handler(invocation: dict) -> None:
    """HITL path. Side-effects only — MUST NOT post a result.

    Writes the pending invocation to ``./pending/<id>.json`` and emits
    a one-line summary to stderr. The human operator reads the file,
    decides on a reply, and runs::

        chakramcp message reply <id> "<reply_text>"

    which sets ``confirmed_by_human: true`` and clears the row.
    """
    inv_id = invocation["id"]
    payload = invocation.get("input_preview") or {}
    message = payload.get("message", "")
    urgency = payload.get("urgency", "normal")
    from_name = payload.get("from_display_name") or invocation.get(
        "grantee_display_name", "(unknown)"
    )

    out_path = pending_dir() / f"{inv_id}.json"
    out_path.write_text(json.dumps(invocation, indent=2, sort_keys=True))

    # One-line summary so a tail -F is enough to monitor traffic.
    print(
        f"[HITL] {inv_id}  from={from_name!r}  urgency={urgency}  "
        f"msg={message[:80]!r}  → {out_path}",
        file=sys.stderr,
        flush=True,
    )


async def ensure_capabilities(chakra: AsyncChakraMCP, agent_id: str) -> None:
    if os.environ.get("CHAKRAMCP_PUBLISH") != "1":
        return
    # Autonomous capability.
    try:
        await chakra.agents.capabilities.create(agent_id, PROPOSE_SLOTS_CAPABILITY)
        print(f"  published: {PROPOSE_SLOTS_CAPABILITY['name']} (autonomous)")
    except ChakraMCPError as err:
        print(f"  publish skipped (propose_slots): {err}", file=sys.stderr)
    # HITL capability — use the template so the schema + semantics are
    # the canonical ones the relay enforces.
    try:
        await chakra.agents.capabilities.add_template(agent_id, "message_owner")
        print(f"  published: {get_template('message_owner')['name']} (human_in_loop)")
    except ChakraMCPError as err:
        print(f"  publish skipped (message_owner): {err}", file=sys.stderr)


def ping_peer_via_cli(peer: str, text: str, from_slug: str) -> None:
    """Outbound bonus: message a peer with cli-v0.1.2's `invoke ensure`.

    Shows the autonomous-orchestration primitive shipped in PR #68 — one
    command discovers the peer, ensures friendship + grant, fires the
    invocation, optionally waits for a terminal result. We use ``--json``
    so the worker can parse the structured response instead of scraping
    stdout.

    Note: ``message_owner`` is HITL on the peer's side, so the result
    will sit ``in_progress`` until their human replies. We wait briefly
    just to demonstrate the JSON envelope shape — a real worker would
    typically fire-and-forget (drop ``--wait``) and pick up the reply
    asynchronously via its own inbox.
    """
    cmd = [
        "chakramcp",
        "invoke",
        "ensure",
        peer,
        "message_owner",
        json.dumps({"message": text, "urgency": "normal"}),
        "--from",
        from_slug,
        "--wait-for-friendship",
        "--wait-for-grant",
        "--json",
    ]
    print(f"  ⇢ outbound: {' '.join(cmd)}", flush=True)
    try:
        proc = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
            timeout=120,
        )
    except FileNotFoundError:
        print(
            "  outbound skipped: `chakramcp` CLI not on PATH "
            "(install with `brew install chakramcp` or `cargo install chakramcp-cli`).",
            file=sys.stderr,
        )
        return
    except subprocess.TimeoutExpired:
        print("  outbound timed out after 120s", file=sys.stderr)
        return

    if proc.returncode != 0:
        # `invoke ensure` exits non-zero on waiting_for_* states too;
        # the JSON body still tells us what happened.
        print(
            f"  outbound exit={proc.returncode} stderr={proc.stderr.strip()[:200]!r}",
            file=sys.stderr,
        )

    try:
        body = json.loads(proc.stdout)
    except json.JSONDecodeError:
        print(f"  outbound: non-JSON stdout: {proc.stdout[:200]!r}", file=sys.stderr)
        return

    inv = body.get("invocation") or {}
    print(
        f"  outbound: ok={body.get('ok')} invocation_id={inv.get('id')} "
        f"status={inv.get('status')}",
        flush=True,
    )


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
    print(f"pending: {pending_dir()}  (HITL drop directory)")
    await ensure_capabilities(chakra, agent_id)

    # Optional outbound demo via the CLI sugar.
    peer = os.environ.get("CHAKRAMCP_PING_PEER")
    from_slug = os.environ.get("CHAKRAMCP_AGENT_SLUG")
    if peer and from_slug:
        ping_peer_via_cli(
            peer,
            os.environ.get("CHAKRAMCP_PING_TEXT", "hello from the reference worker"),
            from_slug,
        )
    elif peer and not from_slug:
        print(
            "  outbound skipped: CHAKRAMCP_PING_PEER set but "
            "CHAKRAMCP_AGENT_SLUG is missing (needed for --from).",
            file=sys.stderr,
        )

    print()
    print("Listening for invocations… (ctrl-c to stop)")
    print()

    stop = asyncio.Event()
    loop = asyncio.get_event_loop()
    try:
        loop.add_signal_handler(signal.SIGINT, stop.set)
        loop.add_signal_handler(signal.SIGTERM, stop.set)
    except NotImplementedError:
        pass

    try:
        await chakra.inbox.serve(
            agent_id,
            handle,
            human_handler=human_handler,
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
        sys.exit(0)
