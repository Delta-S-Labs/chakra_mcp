"""Textual UI — two tabs, push-to-talk, live tool/LLM/MCP log tree.

Tab 1 (Agent): rolling transcript of what the human said + what the
              agent replied. Shown as captions because the actual reply
              is *spoken* via Sarvam TTS.
Tab 2 (Logs):  expandable tree of every span the SDK emits. Each row
              is one LLM/MCP/function call; expanding it reveals the
              arguments and the result. Colors by kind so MCP calls
              pop on camera.

Press SPACE to start recording, release to stop + send. The transcript
flows into the Agent tab, the agent runs, and its final spoken reply
is played back.

We deliberately don't do streaming partials — single-utterance turns
are clearer on camera and make the Logs tab tree one-trace-per-turn
which reads beautifully on screen.
"""

from __future__ import annotations

import asyncio
import json
from typing import Any

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Container, Horizontal, Vertical
from textual.reactive import reactive
from textual.widgets import Footer, Header, Label, Static, TabbedContent, TabPane, Tree

from .agent import AgentStack
from .logs import LogEvent
from .voice import Recorder, SarvamClient, play_wav_bytes_async


# ─── Helpers ────────────────────────────────────────────────────


def _kind_emoji(kind: str) -> str:
    return {
        "llm": "🧠",
        "mcp": "🛰",
        "function": "🔧",
        "agent": "👤",
        "generic": "·",
    }.get(kind, "·")


def _pretty_json(v: Any) -> str:
    if v is None:
        return "(none)"
    if isinstance(v, str):
        return v
    try:
        return json.dumps(v, indent=2, ensure_ascii=False, default=str)
    except Exception:
        return repr(v)


# ─── Widgets ────────────────────────────────────────────────────


class TranscriptView(Static):
    """The Agent tab content — captions only.

    Lines build up over time. The Static widget rerenders the whole
    block per update; that's fine at human-speech cadence.
    """

    lines: reactive[list[str]] = reactive(list)

    def render(self) -> str:
        if not self.lines:
            return "[dim]Hold [b]space[/b] to talk. Release to send.[/dim]"
        return "\n".join(self.lines[-200:])  # keep memory bounded

    def add_line(self, text: str) -> None:
        self.lines = [*self.lines, text]


class LogsView(Vertical):
    """The Logs tab content — an expandable Tree.

    Each span_start adds a node under its parent (or root if it has
    none — top-level traces). Each span_end fills in duration + the
    captured input/output as collapsed children.
    """

    def __init__(self) -> None:
        super().__init__()
        # NB: must NOT be named `_nodes` — that's Textual's internal child
        # NodeList. Our span_id→TreeNode map needs a private name of its own.
        self._span_nodes: dict[str, Any] = {}  # span_id → TreeNode
        self._tree = Tree("Run log", id="run-log")
        self._tree.root.expand()

    def compose(self) -> ComposeResult:
        yield self._tree

    # --- public API used by the App ---
    # NB: must NOT be named `on_event` — that's Textual's built-in event
    # dispatcher, which the framework calls with its own Event objects
    # (Compose, Mount, …). Overriding it routed framework events into
    # this dict-handling code → `'Compose' object has no attribute 'get'`.

    def handle_log(self, msg: dict[str, Any]) -> None:
        t = msg.get("type")
        if t == "trace_start":
            n = self._tree.root.add(f"▶ trace: {msg['name']}", expand=True)
            # We use the trace name as a synthetic id so children whose
            # parent_id is the trace_id can still find a home.
            self._span_nodes[f"trace:{msg['name']}"] = n
        elif t == "trace_end":
            n = self._span_nodes.pop(f"trace:{msg['name']}", None)
            if n is not None:
                n.label = f"✓ trace: {msg['name']}"
        elif t == "span_start":
            ev: LogEvent = msg["event"]
            parent = self._span_nodes.get(ev.parent_id or "", self._tree.root)
            label = f"{_kind_emoji(ev.kind)}  {ev.label}  [dim](starting…)[/dim]"
            node = parent.add(label, expand=False)
            self._span_nodes[ev.span_id] = node
        elif t == "span_end":
            ev = msg["event"]
            node = self._span_nodes.get(ev.span_id)
            if node is None:
                return
            dur = f"{ev.duration_ms} ms" if ev.duration_ms is not None else "?"
            node.label = f"{_kind_emoji(ev.kind)}  {ev.label}  [dim]{dur}[/dim]"
            # Detail children: input, output, anything else worth showing.
            details = ev.details or {}
            for key in ("input", "output", "usage", "name", "model"):
                if key in details and details[key] not in (None, "", [], {}):
                    pretty = _pretty_json(details[key])
                    head = f"[bold]{key}[/bold]"
                    child = node.add(head, expand=False)
                    # Long bodies → second-level leaf; short ones inline.
                    if len(pretty.splitlines()) > 1 or len(pretty) > 80:
                        child.add_leaf(pretty)
                    else:
                        child.label = f"{head}: {pretty}"
            # Auto-scroll to the latest end event so the camera always
            # sees the freshest action.
            self._tree.scroll_end(animate=False)


# ─── The App ────────────────────────────────────────────────────


class VoiceAgentApp(App):
    """Two-tab voice-agent UI.

    Wires:
      * key bindings (space = push-to-talk, q = quit)
      * the SarvamClient + Recorder for voice I/O
      * the AgentStack for the actual loop
      * an asyncio.Queue drained from the log processor
    """

    CSS = """
    Screen {
        layout: vertical;
    }
    #header {
        dock: top;
        height: 3;
        padding: 0 1;
        background: $primary 30%;
    }
    #status {
        dock: bottom;
        height: 1;
        padding: 0 1;
        background: $panel;
        color: $text-muted;
    }
    TranscriptView {
        padding: 1 2;
    }
    LogsView {
        padding: 0;
    }
    Tree > .tree--cursor {
        background: $accent 25%;
    }
    """

    BINDINGS = [
        Binding("space", "ptt_start", "Talk (hold)", show=True, priority=True),
        Binding("q", "quit", "Quit", show=True),
    ]

    status: reactive[str] = reactive("ready · hold space to talk")

    def __init__(
        self,
        stack: AgentStack,
        sarvam: SarvamClient,
        log_queue: asyncio.Queue,
    ) -> None:
        super().__init__()
        self.stack = stack
        self.sarvam = sarvam
        self.log_queue = log_queue
        self.recorder = Recorder()
        self._recording = False
        self._busy = False  # blocks recursive ptt while a turn is in flight
        self._transcript: TranscriptView | None = None
        self._logs: LogsView | None = None

    # ---- Layout -------------------------------------------------------

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        persona = self.stack.persona
        yield Container(
            Label(
                f"[b]{persona.agent_display_name}[/b]   "
                f"acting for [b]{persona.display_name}[/b]   "
                f"· slug [b cyan]{persona.agent_slug}[/b cyan]"
            ),
            id="header",
        )
        with TabbedContent(initial="agent"):
            with TabPane("Agent", id="agent"):
                self._transcript = TranscriptView()
                yield self._transcript
            with TabPane("Logs", id="logs"):
                self._logs = LogsView()
                yield self._logs
        yield Static(self.status, id="status")
        yield Footer()

    def watch_status(self, status: str) -> None:
        try:
            self.query_one("#status", Static).update(status)
        except Exception:
            pass

    # ---- App lifecycle ------------------------------------------------

    async def on_mount(self) -> None:
        # Pump the log queue into the LogsView forever.
        self.run_worker(self._drain_logs(), exclusive=False)
        # Background: poll the relay's inbox so we surface incoming
        # friend requests + invocations without the user pressing space.
        self.run_worker(self._inbox_loop(), exclusive=False, name="inbox")

    async def _drain_logs(self) -> None:
        assert self._logs is not None
        while True:
            msg = await self.log_queue.get()
            try:
                self._logs.handle_log(msg)
            except Exception as e:  # never let a render bug crash the app
                self._logs._tree.root.add(f"[red]log render error: {e}[/red]")

    async def _inbox_loop(self) -> None:
        """Periodically poke the agent with 'check your inbox' so it
        surfaces incoming friend proposals + invocations proactively.

        The agent's instructions tell it to handle inbox items by
        either resolving them (e.g. running an invocation) or
        notifying its owner via update_owner_status.
        """
        await asyncio.sleep(2)  # let startup settle
        while True:
            if not self._busy:
                try:
                    self._busy = True
                    self.status = "checking inbox…"
                    reply = await self.stack.run_turn(
                        "Check your relay inbox. If a friendship was proposed "
                        "and the owner hasn't approved it yet, notify the "
                        "owner via update_owner_status with the proposer's "
                        "agent + display name + the capability they're "
                        "asking for. If an autonomous invocation is pending, "
                        "handle it directly. If nothing's new, reply with "
                        "an empty string."
                    )
                    if reply:
                        await self._on_agent_reply(reply, speak=True)
                except Exception as e:
                    self.status = f"inbox error: {e}"
                finally:
                    self._busy = False
            await asyncio.sleep(8)

    # ---- Push-to-talk -------------------------------------------------

    async def action_ptt_start(self) -> None:
        if self._busy or self._recording:
            return
        self._recording = True
        self.status = "🎙 recording — release space to send"
        try:
            self.recorder.start()
        except Exception as e:
            self.status = f"mic error: {e}"
            self._recording = False
            return

    async def on_key(self, event) -> None:
        # Textual fires key events for both press and release on some
        # backends; we treat the next 'space' after start as "stop".
        # Simpler: use a short timer-based heuristic. Press → record.
        # Release detection on most terminals is unreliable, so we
        # actually stop on the NEXT key event after recording begins —
        # any key — which fits the on-camera flow (a single press-then-
        # release-then-key gesture).
        if not self._recording:
            return
        if event.key == "space":
            self._recording = False
            wav = self.recorder.stop()
            event.stop()
            asyncio.create_task(self._handle_utterance(wav))

    async def _handle_utterance(self, wav: bytes) -> None:
        if self._busy:
            return
        if not wav:
            self.status = "(nothing recorded)"
            return
        self._busy = True
        try:
            self.status = "transcribing…"
            text = await self.sarvam.transcribe(wav)
            if not text:
                self.status = "(silence)"
                return
            assert self._transcript is not None
            self._transcript.add_line(
                f"[b]{self.stack.persona.display_name}:[/b] {text}"
            )
            self.status = "thinking…"
            reply = await self.stack.run_turn(text)
            await self._on_agent_reply(reply, speak=True)
        except Exception as e:
            self.status = f"error: {e}"
        finally:
            self._busy = False
            self.status = "ready · hold space to talk"

    # ---- Agent → owner channel ----------------------------------------

    async def _on_agent_reply(self, text: str, *, speak: bool) -> None:
        if not text:
            return
        assert self._transcript is not None
        self._transcript.add_line(
            f"[b cyan]{self.stack.persona.agent_display_name}:[/b cyan] {text}"
        )
        if speak:
            self.status = "speaking…"
            audio = await self.sarvam.synthesize(text)
            await play_wav_bytes_async(audio)

    def notifier_sync(self, text: str) -> None:
        """The sync entry point passed to make_local_tools.

        The agent calls `update_owner_status(text)`, which lands here
        via _maybe_await. We schedule the actual UI/voice work on the
        app's event loop.
        """
        # Called from tool-execution context; safe to schedule.
        self.call_from_thread(
            lambda: asyncio.create_task(self._on_agent_reply(text, speak=True))
        )
