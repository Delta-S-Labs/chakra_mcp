"""Hermes — the pull-mode side of the demo.

Runs `inbox.serve()` against the relay, answering `answer_question`
invocations from anyone who's been granted access. This is the
ChakraMCP-native flow: no public host needed; Hermes long-polls the
relay's `/v1/inbox` endpoint and pushes responses back via
`POST /v1/invocations/<id>/result`.

Two run modes:

  python hermes_bot.py            # long-running serve loop (default)
  python hermes_bot.py --once     # cron-friendly: drain inbox + exit

The `--once` mode is what you'd put behind a `cron` / systemd timer /
GitHub Actions schedule. It uses `inbox.pull()` to grab whatever's
waiting, answers each one, and exits with code 0 on success or 1 on
any per-invocation error.

Stop the long-running mode with ctrl-c.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import signal
import sys
from pathlib import Path

from chakramcp import AsyncChakraMCP

STATE = json.loads((Path(__file__).parent / "state.json").read_text())


# ─── Toy "knowledge base" ─────────────────────────────────────────
# Replace with an LLM call, a DB, or a vector store. The demo's
# point is the relay flow, not the answer quality.

CANNED_ANSWERS = {
    "weather": "Sunny with a chance of relay traffic.",
    "time": "Time is a flat circle, but the relay's clock is monotonic.",
    "recipe": "I don't cook — try the OpenClaw agent next door.",
    "openclaw": (
        "OpenClaw is a push-mode A2A agent. The relay fetches its Agent "
        "Card and forwards JSONRPC calls to its public endpoint."
    ),
    "chakramcp": (
        "ChakraMCP is the relay you're talking through right now. It "
        "handles friendships, grants, audit logs, and A2A protocol "
        "translation between native and external agents."
    ),
}


def answer(question: str, style: str = "concise") -> str:
    """Pick a canned answer by keyword match. Fallback echoes the
    question to make it obvious in logs that the loop is alive."""
    q = question.lower()
    for kw, reply in CANNED_ANSWERS.items():
        if kw in q:
            if style == "verbose":
                return f"On '{question}': {reply}"
            return reply
    return f"(echo) you asked: {question!r}"


# ─── Invocation handler ───────────────────────────────────────────


async def handle(invocation: dict) -> dict:
    """Handle a single invocation row delivered by the relay.

    The relay has already verified the friendship + grant before
    handing this row to us. We can use `grant_context` and
    `friendship_context` for audit / logging without re-querying.
    """
    capability = invocation["capability_name"]
    inputs = invocation.get("input_preview") or {}

    grant = invocation.get("grant_context") or {}
    friendship = invocation.get("friendship_context") or {}

    print(f"  ← {capability}({inputs})")
    if grant:
        print(
            f"    grant {grant.get('id', '')[:13]}…  "
            f"granter={grant.get('granter_agent_slug')}  "
            f"grantee={grant.get('grantee_agent_slug')}"
        )
    if friendship:
        msg = friendship.get("proposer_message") or "—"
        print(f"    friendship initial msg: {msg!r}")

    if capability != "answer_question":
        return {
            "status": "failed",
            "error": f"unsupported capability: {capability}",
        }

    question = str(inputs.get("question", "")).strip()
    if not question:
        return {"status": "failed", "error": "missing 'question' in input"}

    style = inputs.get("style", "concise")
    reply = answer(question, style=style)

    print(f"  → answered ({len(reply)} chars)")
    return {
        "status": "succeeded",
        "output": {"answer": reply, "from": "hermes"},
    }


# ─── One-shot cron mode ───────────────────────────────────────────


async def run_once(chakra: AsyncChakraMCP, agent_id: str) -> int:
    """Drain whatever's in the inbox right now, then exit.

    Designed for `cron` / systemd `OnCalendar=` / GitHub Actions
    `schedule:`. Returns 0 if every invocation handled cleanly, 1 if
    any errored. A relay that's responsive but empty returns 0.
    """
    invocations = await chakra.inbox.pull(agent_id, limit=25)
    if not invocations:
        print("  inbox empty — exiting cleanly")
        return 0

    print(f"  pulled {len(invocations)} invocation(s)")
    failed = 0
    for inv in invocations:
        try:
            result = await handle(inv)
            await chakra.inbox.respond(inv["id"], result)
        except Exception as err:  # noqa: BLE001
            print(f"  ! error on {inv.get('id')}: {err}")
            try:
                await chakra.inbox.respond(
                    inv["id"], {"status": "failed", "error": str(err)}
                )
            except Exception:  # noqa: BLE001
                pass
            failed += 1
    return 0 if failed == 0 else 1


# ─── Long-running serve mode ─────────────────────────────────────


async def run_forever(chakra: AsyncChakraMCP, agent_id: str) -> None:
    print("Listening for invocations… (ctrl-c to stop)")
    print()

    stop = asyncio.Event()
    loop = asyncio.get_event_loop()
    try:
        loop.add_signal_handler(signal.SIGINT, stop.set)
        loop.add_signal_handler(signal.SIGTERM, stop.set)
    except NotImplementedError:
        pass  # Windows; ctrl-c still raises KeyboardInterrupt

    await chakra.inbox.serve(
        agent_id,
        handle,
        poll_interval_s=1.0,
        stop_event=stop,
        on_error=lambda err, inv: print(
            f"  ! error: {err} (inv={inv and inv.get('id')})"
        ),
    )


async def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument(
        "--once",
        action="store_true",
        help="Drain the inbox once and exit (cron-friendly).",
    )
    args = parser.parse_args()

    chakra = AsyncChakraMCP(
        api_key=STATE["hermes"]["api_key"],
        app_url=STATE["app_url"],
        relay_url=STATE["relay_url"],
    )

    me = await chakra.me()
    print(f"signed in as {me['user']['email']}")
    print(f"agent  : {STATE['hermes']['agent_id']}  ({STATE['hermes']['agent_slug']})")
    print()

    try:
        if args.once:
            return await run_once(chakra, STATE["hermes"]["agent_id"])
        await run_forever(chakra, STATE["hermes"]["agent_id"])
    finally:
        await chakra.aclose()

    print()
    print("stopped.")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
