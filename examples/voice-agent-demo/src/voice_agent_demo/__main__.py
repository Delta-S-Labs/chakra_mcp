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
from pathlib import Path

import click
from dotenv import load_dotenv

from agents import add_trace_processor

from .agent import build_agent_stack
from .app import VoiceAgentApp
from .chakra_mcp import build_chakra_mcp_server, load_cli_session
from .logs import QueueProcessor
from .persona import Persona
from .swiggy_mcp import build_swiggy_mcp_server
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
    # Voice — fail fast if Sarvam isn't configured.
    voice_cfg = voice_config_from_env()
    sarvam = SarvamClient(voice_cfg)

    # ChakraMCP — relies on `chakramcp login`.
    try:
        session = load_cli_session()
    except (FileNotFoundError, RuntimeError) as e:
        click.echo(f"\n  ✗ ChakraMCP CLI not signed in: {e}\n", err=True)
        click.echo("    Run `chakramcp login` first, then re-run this command.\n", err=True)
        sys.exit(2)

    chakra_mcp = build_chakra_mcp_server(session)
    swiggy_mcp = build_swiggy_mcp_server()  # None if env not set

    # Tracing — register BEFORE entering the MCP servers so the very
    # first list_tools call shows up in the Logs tab.
    log_queue: asyncio.Queue = asyncio.Queue()
    processor = QueueProcessor()
    processor.attach(log_queue, asyncio.get_running_loop())
    add_trace_processor(processor)

    # The MCP clients need to be entered (connection opened) before the
    # agent can call tools through them. The OpenAI SDK accepts them as
    # async context managers; we use ExitStack-style nesting.
    async with chakra_mcp:
        cm_swiggy = swiggy_mcp if swiggy_mcp is not None else _NullCtx()
        async with cm_swiggy:
            # Build the agent. Owner-notification is a forward reference
            # to the Textual app, which doesn't exist yet — we plumb a
            # closure that the app re-binds on mount.
            app_ref: dict[str, VoiceAgentApp | None] = {"app": None}

            def notifier(text: str):
                a = app_ref["app"]
                if a is None:
                    return None
                a.notifier_sync(text)
                return None

            stack = await build_agent_stack(
                persona, chakra_mcp, swiggy_mcp, notifier
            )

            app = VoiceAgentApp(stack=stack, sarvam=sarvam, log_queue=log_queue)
            app_ref["app"] = app
            try:
                await app.run_async()
            finally:
                await sarvam.aclose()


class _NullCtx:
    async def __aenter__(self):
        return None
    async def __aexit__(self, *a):
        return False


if __name__ == "__main__":
    cli()
