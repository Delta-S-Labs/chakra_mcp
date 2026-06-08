"""CLI entry — `uv run voice-agent --persona kaustav`.

Order of bring-up:

  1. Load .env + parse --persona.
  2. Read the CLI's session (the JWT minted by `chakramcp login`).
  3. Build the two MCP servers (ChakraMCP required, Swiggy optional).
  4. Register our TracingProcessor *before* any agent runs, so even
     the first MCP list-tools call shows up in the Logs tab.
  5. Enter the MCP servers (async-with style — they need an open
     connection for the agent to call tools through them).
  6. Construct the Agent + AgentStack.
  7. Start Textual.
"""

from __future__ import annotations

import asyncio
import sys
from contextlib import AsyncExitStack
from dataclasses import replace
from pathlib import Path

import click
from dotenv import load_dotenv

from agents import set_trace_processors

from .agent import build_agent_stack, ensure_capabilities, llm_label
from .app import VoiceAgentApp
from .chakra_mcp import build_chakra_mcp_server, load_cli_session, resolve_agent_id
from .logs import QueueProcessor
from .persona import Persona
from .swiggy_auth import ensure_swiggy_token
from .swiggy_mcp import build_swiggy_mcp_server, swiggy_mcp_url
from .voice import SarvamClient, voice_config_from_env


@click.command()
@click.option(
    "--persona",
    required=True,
    help="Persona name — must match a JSON in personas/ (e.g. kaustav, aparajita).",
)
@click.option(
    "--env",
    "env_path",
    type=click.Path(exists=False),
    default=None,
    help="Path to .env (default: ./examples/voice-agent-demo/.env).",
)
def cli(persona: str, env_path: str | None) -> None:
    # .env — let users run from the repo root or from this folder.
    if env_path:
        load_dotenv(env_path)
    else:
        # Search upwards for the demo's .env.
        for candidate in (
            Path.cwd() / "examples/voice-agent-demo/.env",
            Path(__file__).resolve().parent.parent.parent / ".env",
            Path.cwd() / ".env",
        ):
            if candidate.exists():
                load_dotenv(candidate)
                break

    p = Persona.load(persona)
    asyncio.run(_run(p))


async def _run(persona: Persona) -> None:
    # Voice — fail fast if Sarvam isn't configured. The persona's own
    # `voice_speaker` (e.g. manan / ritu) wins over the env default so each
    # agent has a distinct voice.
    voice_cfg = voice_config_from_env()
    if persona.voice_speaker:
        voice_cfg = replace(voice_cfg, speaker=persona.voice_speaker)
    sarvam = SarvamClient(voice_cfg)
    click.echo(f"  · voice: {voice_cfg.model} / speaker {voice_cfg.speaker}")

    # ChakraMCP — relies on `chakramcp login`.
    try:
        session = load_cli_session()
    except (FileNotFoundError, RuntimeError) as e:
        click.echo(f"\n  ✗ ChakraMCP CLI not signed in: {e}\n", err=True)
        click.echo("    Run `chakramcp login` first, then re-run this command.\n", err=True)
        sys.exit(2)

    chakra_mcp = build_chakra_mcp_server(session)

    # Swiggy — OAuth 2.1 at startup (before the TUI grabs the screen, so
    # the browser handoff is clean). First run on each laptop opens a
    # browser; later runs reuse the cached ~5-day token. If the user
    # hasn't configured Swiggy (no SWIGGY_MCP_URL) or auth fails, we run
    # without it and the agent degrades gracefully.
    swiggy_token: str | None = None
    if swiggy_mcp_url():
        click.echo("  · Authenticating Swiggy Dineout (OAuth 2.1)…")
        try:
            swiggy_token = ensure_swiggy_token(
                persona.name, swiggy_mcp_url(), on_status=lambda s: click.echo(f"    {s}")
            )
        except Exception as e:
            click.echo(f"  ! Swiggy auth failed ({e}); continuing without Swiggy.", err=True)
    swiggy_mcp = build_swiggy_mcp_server(swiggy_token)

    # Tracing — register BEFORE entering the MCP servers so the very
    # first list_tools call shows up in the Logs tab. We REPLACE the
    # default processors (set_, not add_) so the SDK's built-in OpenAI
    # trace exporter is removed: it would otherwise try to upload traces
    # to OpenAI, which fails/ warns when running on Groq with no
    # OPENAI_API_KEY. Our local QueueProcessor still feeds the Logs tab.
    log_queue: asyncio.Queue = asyncio.Queue()
    processor = QueueProcessor()
    processor.attach(log_queue, asyncio.get_running_loop())
    set_trace_processors([processor])
    click.echo(f"  · LLM: {llm_label()}")

    # The MCP clients need their connection open before the agent can
    # call tools through them. ChakraMCP is required — a failure there is
    # fatal (no relay = no demo). Swiggy is OPTIONAL: if its MCP session
    # won't establish (auth edge, wrong path, server hiccup), we drop it
    # and the agent degrades to a verbal restaurant suggestion rather
    # than crashing the whole app on startup.
    async with AsyncExitStack() as mcp_stack:
        await mcp_stack.enter_async_context(chakra_mcp)

        # Resolve THIS persona's own relay agent id up front. Every relay
        # interaction tool (pull_inbox, respond, invoke) requires it, and
        # the LLM can't infer it — so we pin it into the system prompt.
        # If the agent isn't registered yet, we surface the fix loudly and
        # keep going in a degraded "needs registration" mode rather than
        # letting the agent spew "missing field agent_id" errors.
        agent_id = await resolve_agent_id(chakra_mcp, persona.agent_slug)
        if agent_id:
            click.echo(f"  ✓ relay agent: {persona.agent_slug} ({agent_id})")
            # The agent may have been created in a prior session before its
            # capabilities were published. Backfill any missing ones now
            # (idempotent) so the demo flow always has negotiate_dinner.
            try:
                pub = await ensure_capabilities(chakra_mcp, agent_id)
                if pub:
                    click.echo(f"  ✓ published capabilities: {', '.join(pub)}")
            except Exception as e:
                click.echo(f"  ! couldn't ensure capabilities ({e})", err=True)
        else:
            click.echo(
                f"\n  ⚠ No relay agent with slug '{persona.agent_slug}' is "
                "registered for this account.\n"
                "    The demo flow (inbox, friendships, invoke) needs one. "
                "Register it with:\n"
                f"      ./scripts/register-agent.sh {persona.name}\n"
                "    Running anyway in degraded mode (voice works; relay "
                "actions will be declined).\n",
                err=True,
            )

        if swiggy_mcp is not None:
            try:
                await mcp_stack.enter_async_context(swiggy_mcp)
            except Exception as e:
                click.echo(
                    f"  ! Swiggy MCP didn't connect ({type(e).__name__}: {e}); "
                    "continuing without it — the agent will suggest a "
                    "restaurant verbally instead.",
                    err=True,
                )
                click.echo(
                    "    (If you want Swiggy live, try a different SWIGGY_MCP_URL "
                    "in .env — e.g. drop the trailing /mcp — and re-run.)",
                    err=True,
                )
                swiggy_mcp = None

        # Build the agent. Owner-notification is a forward reference to
        # the Textual app, which doesn't exist yet — we plumb a closure
        # that the app re-binds on mount.
        app_ref: dict[str, VoiceAgentApp | None] = {"app": None}

        def notifier(text: str):
            a = app_ref["app"]
            if a is None:
                return None
            a.notifier_sync(text)
            return None

        stack = await build_agent_stack(
            persona, chakra_mcp, swiggy_mcp, notifier, agent_id
        )

        app = VoiceAgentApp(stack=stack, sarvam=sarvam, log_queue=log_queue)
        app_ref["app"] = app
        try:
            await app.run_async()
        finally:
            # Watchdog: once the TUI has closed, GUARANTEE the process
            # dies even if an MCP server's or HTTP client's async cleanup
            # hangs (which left the old process unkillable). Fires only if
            # graceful teardown — including the AsyncExitStack closing the
            # MCP servers after this block — doesn't finish in time.
            import os
            import threading

            watchdog = threading.Timer(6.0, lambda: os._exit(0))
            watchdog.daemon = True
            watchdog.start()
            try:
                await asyncio.wait_for(sarvam.aclose(), timeout=5.0)
            except Exception:
                pass


if __name__ == "__main__":
    cli()
