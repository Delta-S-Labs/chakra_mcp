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

from rich.markup import escape
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Container, Horizontal, Vertical
from textual.reactive import reactive
from textual.widgets import Footer, Header, Label, Static, TabbedContent, TabPane, Tree

from .agent import AgentStack
from .chakra_mcp import call_tool_json
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
            return (
                "[dim]Press [b]space[/b] to start talking, [b]space[/b] again to "
                "send.\nYour words appear here as [b]"
                f"{self._owner_name}[/b]: …, the agent replies below.[/dim]"
            )
        return "\n".join(self.lines[-200:])  # keep memory bounded

    _owner_name: str = "you"

    def add_line(self, text: str) -> None:
        self.lines = [*self.lines, text]


class PttIndicator(Static):
    """Big, always-visible voice-state bar so the operator (and the
    camera) can tell at a glance what the agent is doing: idle,
    LISTENING (recording), transcribing, thinking, or speaking.

    The `frame` counter is bumped by a timer on the App; we animate an
    equalizer so "listening" and "speaking" read as live, not frozen.
    """

    phase: reactive[str] = reactive("idle")
    frame: reactive[int] = reactive(0)

    _BARS = "▁▂▃▄▅▆▇█▇▆▅▄▃▂"

    def _eq(self, step: int, width: int = 16) -> str:
        n = len(self._BARS)
        return "".join(self._BARS[(self.frame * step + i) % n] for i in range(width))

    def render(self) -> str:
        p = self.phase
        if p == "listening":
            return (
                f"[b white on red] ● REC [/b white on red]  "
                f"[green]{self._eq(1)}[/green]   "
                "[dim]speak now — press [b]SPACE[/b] again to send[/dim]"
            )
        if p == "transcribing":
            dots = "." * (1 + self.frame % 3)
            return f"[b yellow]✍  transcribing{dots}[/b yellow]"
        if p == "thinking":
            dots = "." * (1 + self.frame % 3)
            return f"[b cyan]🤔 thinking{dots}[/b cyan]"
        if p == "speaking":
            return f"[b magenta]🔊 speaking[/b magenta]  [magenta]{self._eq(2)}[/magenta]"
        return "[dim]●[/dim] [b]ready[/b] — press [b green]SPACE[/b green] to talk, [b]SPACE[/b] again to send"


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
            # Render EVERY meaningful detail, not a fixed allowlist — the
            # LLM `output_text`/`response` and list_tools `result` are the
            # interesting bits. Preferred keys first, then any extras.
            details = ev.details or {}
            preferred = ["output_text", "input", "output", "result", "usage", "name", "model"]
            ordered = [k for k in preferred if k in details]
            ordered += [k for k in details if k not in preferred]
            for key in ordered:
                value = details[key]
                if value in (None, "", [], {}):
                    continue
                self._add_detail(node, key, value)
            # Auto-scroll to the latest end event so the camera always
            # sees the freshest action.
            self._tree.scroll_end(animate=False)

    def _add_detail(self, node: Any, key: str, value: Any) -> None:
        """Attach one detail to a span node. Short scalars render inline;
        multi-line JSON is expanded to one leaf PER LINE, because a
        Textual Tree node label is single-line — dumping a whole blob in
        one leaf only ever shows its first character (`[` or `{`)."""
        pretty = _pretty_json(value)
        lines = pretty.splitlines() or [pretty]
        head = f"[bold]{key}[/bold]"
        if len(lines) == 1 and len(pretty) <= 80:
            # Escape the value — JSON contains [ ] { } which Rich would
            # otherwise try to parse as console markup.
            node.add_leaf(f"{head}: {escape(pretty)}")
            return
        branch = node.add(head, expand=False)
        MAX_LINES = 400  # guard against a giant message-history dump
        for line in lines[:MAX_LINES]:
            # Tree leaves need a non-empty label; keep indentation intact.
            branch.add_leaf(escape(line) if line.strip() else " ")
        if len(lines) > MAX_LINES:
            branch.add_leaf(f"[dim]… ({len(lines) - MAX_LINES} more lines)[/dim]")


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
    #ptt {
        dock: top;
        height: 1;
        padding: 0 1;
        background: $boost;
        text-style: bold;
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
        Binding("space", "ptt_toggle", "Talk / send", show=True, priority=True),
        Binding("q", "quit", "Quit", show=True, priority=True),
        # Make Ctrl-C / Ctrl-Q reliably quit too (Textual captures the
        # terminal, so a bare ^C is a key event, not SIGINT).
        Binding("ctrl+c", "quit", "Quit", show=False, priority=True),
        Binding("ctrl+q", "quit", "Quit", show=False, priority=True),
    ]

    status: reactive[str] = reactive("ready · press space to talk")

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
        self._busy = False  # a turn (user or inbox) is in flight
        self._inbox_tick = 0
        self._transcript: TranscriptView | None = None
        self._logs: LogsView | None = None
        self._ptt: PttIndicator | None = None

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
        self._ptt = PttIndicator(id="ptt")
        yield self._ptt
        with TabbedContent(initial="agent"):
            with TabPane("Agent", id="agent"):
                self._transcript = TranscriptView()
                self._transcript._owner_name = persona.display_name
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
        # Drive the PTT indicator's equalizer animation (~8 fps).
        self.set_interval(0.12, self._animate_ptt)
        # Pump the log queue into the LogsView forever.
        self.run_worker(self._drain_logs(), exclusive=False)
        # Background: poll the relay's inbox so we surface incoming
        # friend requests + invocations without the user pressing space.
        self.run_worker(self._inbox_loop(), exclusive=False, name="inbox")

    def on_unmount(self) -> None:
        # Make sure the mic stream is released on quit so PortAudio
        # doesn't keep the process alive.
        try:
            if self._recording:
                self.recorder.stop()
        except Exception:
            pass

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
        await asyncio.sleep(3)  # let startup settle
        while True:
            await asyncio.sleep(12)
            # The user ALWAYS has priority: never run a background turn
            # while they're recording or a turn is already in flight.
            # (This is what kept push-to-talk dead — the old loop held
            # _busy for seconds every 8s, swallowing space presses.)
            if self._recording or self._busy:
                continue
            agent_id = self.stack.agent_id
            if agent_id is None:
                continue  # not registered yet — nothing to poll

            # Cheap pre-check (no LLM): is there a pending inbound friend
            # request? If so, handle this tick. Otherwise only run a full
            # serve-turn occasionally (~every 60s) to catch invocations,
            # so we don't burn an LLM call (and block the user) each tick.
            self._inbox_tick += 1
            try:
                pending_friend = await self._has_pending_friend(agent_id)
            except Exception as e:
                self.status = f"inbox poll error: {e}"
                continue
            if not (pending_friend or self._inbox_tick % 5 == 0):
                continue
            # Re-check the user didn't just start talking.
            if self._recording or self._busy:
                continue

            try:
                self._busy = True
                self.status = "checking inbox…"
                reply = await self.stack.run_turn(
                    f'Check your relay inbox by calling pull_inbox with '
                    f'agent_id="{agent_id}". If a friendship was proposed and '
                    "the owner hasn't approved it yet, notify the owner via "
                    "update_owner_status with the proposer's agent + display "
                    "name + the capability they're asking for. If an "
                    "autonomous invocation is pending, handle it directly "
                    f'(respond with the same agent_id="{agent_id}"). If '
                    "nothing's new, reply with an empty string."
                )
                if reply:
                    await self._on_agent_reply(reply, speak=True)
            except Exception as e:
                self.status = f"inbox error: {e}"
            finally:
                self._busy = False
                if not self._recording:
                    self._set_phase("idle")
                    self.status = "ready · press space to talk"

    async def _has_pending_friend(self, agent_id: str) -> bool:
        """Cheap, no-LLM, no-claim check for an inbound proposed friendship."""
        fr = await call_tool_json(
            self.stack.chakra_mcp,
            "list_friendships",
            {"direction": "inbound", "status": "proposed"},
        )
        return bool(isinstance(fr, list) and fr)

    # ---- Push-to-talk -------------------------------------------------

    def _animate_ptt(self) -> None:
        # Only advance frames when something is animating, to keep the
        # idle/ready bar perfectly still.
        if self._ptt is not None and self._ptt.phase != "idle":
            self._ptt.frame += 1

    def _set_phase(self, phase: str) -> None:
        if self._ptt is not None:
            self._ptt.frame = 0
            self._ptt.phase = phase

    async def action_ptt_toggle(self) -> None:
        """SPACE toggles recording.

        This MUST be a single toggling action, not a start-action plus an
        on_key stop-handler: the `space` binding is `priority=True`, so
        Textual consumes the key and dispatches it to this action — the
        app's `on_key` never sees it. So the same action handles both the
        start press and the send press.
        """
        if self._recording:
            # Second press → stop + send. ALWAYS allowed, even if a
            # background turn is busy — otherwise a recording can get
            # stuck "on" when the inbox loop flips _busy mid-utterance.
            self._recording = False
            try:
                wav = self.recorder.stop()
            except Exception as e:
                self._set_phase("idle")
                self.status = f"mic error: {e}"
                return
            asyncio.create_task(self._handle_utterance(wav))
            return

        # First press → start. Blocked only while a turn is in flight.
        if self._busy:
            self.status = "busy — one moment…"
            return
        try:
            self.recorder.start()
        except Exception as e:
            self._set_phase("idle")
            self.status = f"mic error: {e}"
            return
        self._recording = True
        self._set_phase("listening")
        self.status = "🎙 recording — press space again to send"

    async def _handle_utterance(self, wav: bytes) -> None:
        if not wav:
            self._set_phase("idle")
            self.status = "(nothing recorded — press space and speak)"
            return
        # A background inbox turn may be running; wait briefly for it to
        # finish rather than dropping the user's speech.
        waited = 0.0
        while self._busy and waited < 30.0:
            self.status = "finishing previous task…"
            await asyncio.sleep(0.2)
            waited += 0.2
        self._busy = True
        try:
            self._set_phase("transcribing")
            self.status = "transcribing…"
            text = await self.sarvam.transcribe(wav)
            if not text:
                self._set_phase("idle")
                self.status = "(heard silence — try again)"
                return
            assert self._transcript is not None
            self._transcript.add_line(
                f"[b]{self.stack.persona.display_name}:[/b] {text}"
            )
            self._set_phase("thinking")
            self.status = "thinking…"
            reply = await self.stack.run_turn(text)
            await self._on_agent_reply(reply, speak=True)
        except Exception as e:
            self.status = f"error: {e}"
        finally:
            self._busy = False
            self._set_phase("idle")
            self.status = "ready · press space to talk"

    # ---- Agent → owner channel ----------------------------------------

    async def _on_agent_reply(self, text: str, *, speak: bool) -> None:
        if not text:
            return
        assert self._transcript is not None
        self._transcript.add_line(
            f"[b cyan]{self.stack.persona.agent_display_name}:[/b cyan] {text}"
        )
        if speak:
            self._set_phase("speaking")
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
