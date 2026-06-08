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
import os
from typing import Any

from rich.markup import escape
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Container, Horizontal, Vertical, VerticalScroll
from textual.reactive import reactive
from textual.widgets import Footer, Header, Label, Static, TabbedContent, TabPane, Tree

from .agent import AgentStack
from .chakra_mcp import call_tool_json
from .logs import LogEvent
from .voice import Recorder, SarvamClient, play_wav_bytes_async, stop_playback

def _env_int(name: str, default: int) -> int:
    try:
        v = int(os.environ.get(name, "") or default)
        return v if v > 0 else default
    except ValueError:
        return default


# Wall-clock ceiling on ONE agent turn. The full relay+negotiation flow on
# a slower reasoning model can legitimately run a couple of minutes, so the
# default is generous; override with AGENT_TURN_TIMEOUT_S. Beyond it we
# assume the turn is wedged and recover rather than freezing the TUI.
TURN_TIMEOUT_S = _env_int("AGENT_TURN_TIMEOUT_S", 900)

# How often the background loop checks the relay for new friend requests
# and pending invocations. Cheap (one or two MCP list calls), so frequent.
POLL_INTERVAL_S = 5


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
    """The Agent tab content — a caption log.

    A plain Static (markup) wrapped in a VerticalScroll. We keep the
    lines ourselves, re-render the joined block, then scroll the parent
    to the bottom so the newest turn is always visible. (RichLog would be
    the obvious choice, but its internals are version-fragile across the
    Textual builds the two laptops resolved — this is robust.)
    """

    _owner_name: str = "you"

    def __init__(self) -> None:
        super().__init__()
        self._lines: list[str] = []

    def on_mount(self) -> None:
        self._rerender()

    def add_line(self, text: str) -> None:
        self._lines.append(text)
        self._rerender()
        # Scroll after layout settles so the latest line is visible.
        self.call_after_refresh(self._scroll_end)

    def _rerender(self) -> None:
        if self._lines:
            self.update("\n".join(self._lines[-300:]))
        else:
            self.update(
                "[dim]Press [b]space[/b] to talk, [b]space[/b] again to send. "
                f"You appear as [b]{self._owner_name}[/b]; the agent replies "
                "below.[/dim]"
            )

    def _scroll_end(self) -> None:
        try:
            self.app.query_one("#agent-scroll", VerticalScroll).scroll_end(animate=False)
        except Exception:
            pass


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
            return (
                f"[b magenta]🔊 speaking[/b magenta]  [magenta]{self._eq(2)}[/magenta]"
                "   [dim]press [b]SPACE[/b] to stop[/dim]"
            )
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
    #agent-scroll {
        height: 1fr;
    }
    TranscriptView {
        padding: 1 2;
        height: auto;
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
        # Serializes ALL agent turns. The inbox loop and a user turn must
        # never run Runner.run concurrently — they share one MCP session,
        # and concurrent calls on a Streamable-HTTP MCP session deadlock.
        self._turn_lock = asyncio.Lock()
        self._speaking = False  # True while TTS audio is playing
        self._stop_speech = False  # set when the user interrupts playback
        # Pending human-in-the-loop (message_owner) invocations claimed from
        # the inbox, surfaced to the owner and awaiting their spoken reply.
        self._pending_owner_msgs: list[dict] = []
        # State so we announce each pending friend request exactly ONCE and
        # don't nag until it's resolved (then a brand-new one re-announces).
        self._announced_friendships: set[str] = set()
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
                yield VerticalScroll(self._transcript, id="agent-scroll")
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
        """Frequent, deterministic relay poll.

        Two channels, both checked every tick:
          * Friend requests (list_friendships inbound/proposed) — surfaced
            to the owner exactly ONCE each (state in _announced_friendships)
            and never re-announced until resolved.
          * Pending invocations (pull_inbox) — served by one LLM turn only
            when something is actually claimed.
        When there's nothing new we stay silent — just a quiet status line,
        no speech, no owner interruption.
        """
        await asyncio.sleep(3)  # let startup settle
        while True:
            await asyncio.sleep(POLL_INTERVAL_S)
            agent_id = self.stack.agent_id
            if agent_id is None:
                continue  # not registered yet — nothing to poll
            try:
                # Hard timeout so a stuck/half-open relay connection can
                # never freeze the background loop (the MCP client also has
                # its own per-call timeout; this is belt-and-suspenders).
                await asyncio.wait_for(self._poll_friend_requests(), timeout=50)
                # Results of calls WE issued may land late (a friend's human
                # answers minutes later). Cheap, no LLM — surface to owner.
                await asyncio.wait_for(
                    self._poll_outgoing_results(), timeout=50
                )
                # Serving an invocation needs the LLM — only when the user
                # isn't mid-turn/recording (they always have priority).
                if not (self._recording or self._busy or self._turn_lock.locked()):
                    # Contains a full guarded turn, so allow it the whole
                    # turn budget plus a little slack.
                    await asyncio.wait_for(
                        self._poll_invocations(agent_id), timeout=TURN_TIMEOUT_S + 30
                    )
            except asyncio.TimeoutError:
                self.status = "· relay slow — will retry"
            except Exception as e:
                # Quiet log only — never nag the owner about poll failures.
                self.status = f"· relay poll error: {e}"

    async def _poll_friend_requests(self) -> None:
        fr = await call_tool_json(
            self.stack.chakra_mcp,
            "list_friendships",
            {"direction": "inbound", "status": "proposed"},
        )
        fr = fr if isinstance(fr, list) else []
        current_ids = {f.get("id") for f in fr}
        # Forget any we'd announced that are no longer pending (accepted /
        # rejected) so a genuinely NEW request later announces again.
        self._announced_friendships &= current_ids

        new = [f for f in fr if f.get("id") not in self._announced_friendships]
        if not new:
            # Nothing new — quiet heartbeat, no speech, no transcript line.
            if not self._busy and not self._recording:
                self.status = "· up to date — press space to talk"
            return

        assert self._transcript is not None
        for f in new:
            self._announced_friendships.add(f.get("id"))
            who = f.get("proposer_display_name") or "An agent"
            line = f"{who} sent you a friend request — say “accept” to approve."
            self._transcript.add_line(f"[b yellow]· {line}[/b yellow]")
            # Speak the heads-up once, but only if we won't talk over the
            # user; otherwise it's already visible in the transcript.
            if not self._recording and not self._busy:
                await self._speak(line)

    async def _poll_outgoing_results(self) -> None:
        """Poll invocations WE issued for late results and tell the owner.

        The relay's `invoke` is fire-and-forget — it returns `pending`,
        and the result may land minutes later (e.g. a friend's human
        answers a message_owner relay). We track issued ids in
        `stack.pending_outgoing` and poll each until it reaches a terminal
        state, then announce it exactly once and stop tracking it.
        """
        outstanding = self.stack.pending_outgoing
        if not outstanding:
            return
        terminal = {"succeeded", "failed", "rejected", "timeout"}
        done: list[str] = []
        for inv_id, meta in list(outstanding.items()):
            try:
                row = await call_tool_json(
                    self.stack.chakra_mcp,
                    "poll_invocation",
                    {"invocation_id": inv_id},
                )
            except Exception:
                continue  # transient — try again next tick
            if not isinstance(row, dict):
                continue
            status = row.get("status")
            if status not in terminal:
                continue  # still pending / in_progress
            done.append(inv_id)
            if meta.get("announced"):
                continue
            meta["announced"] = True
            cap = (
                row.get("capability_name")
                or meta.get("capability_name")
                or "a request"
            )
            if status == "succeeded":
                out = row.get("output_preview") or {}
                reply = (
                    (out.get("reply") or out.get("text") or out.get("message"))
                    if isinstance(out, dict)
                    else None
                ) or (json.dumps(out)[:300] if out else "done")
                line = f"✅ Reply to your “{cap}” request: {reply}"
            else:
                err = row.get("error_message") or status
                line = f"⚠ Your “{cap}” request {status}: {err}"
            if self._transcript is not None:
                self._transcript.add_line(f"[b green]· {line}[/b green]")
            if not self._recording and not self._busy:
                await self._speak(line)
        for inv_id in done:
            outstanding.pop(inv_id, None)

    async def _poll_invocations(self, agent_id: str) -> None:
        # pull_inbox CLAIMS pending invocations (→ in_progress), so this is
        # self-deduping: once pulled they won't reappear. Only spin up the
        # LLM when something was actually claimed.
        inbox = await call_tool_json(
            self.stack.chakra_mcp, "pull_inbox", {"agent_id": agent_id}
        )
        inbox = inbox if isinstance(inbox, list) else []
        if not inbox:
            return

        # Partition: message_owner is human-in-the-loop — surface it to the
        # owner and DON'T let the LLM answer it. Everything else is
        # autonomous and gets served by an LLM turn.
        hitl = [i for i in inbox if i.get("capability_name") == "message_owner"]
        auto = [i for i in inbox if i.get("capability_name") != "message_owner"]

        for item in hitl:
            inv_id = item.get("invocation_id")
            if not inv_id or any(m["invocation_id"] == inv_id for m in self._pending_owner_msgs):
                continue
            who = item.get("grantee_display_name") or "a friend's agent"
            inp = item.get("input") or {}
            msg = (
                (inp.get("text") or inp.get("message") or inp.get("reply"))
                if isinstance(inp, dict)
                else None
            ) or (json.dumps(inp) if inp else "(no message)")
            self._pending_owner_msgs.append(
                {"invocation_id": inv_id, "from": who, "message": msg}
            )
            line = f"📨 {who} asks: {msg} — say your reply and I'll send it."
            if self._transcript is not None:
                self._transcript.add_line(f"[b yellow]· {line}[/b yellow]")
            if not self._recording and not self._busy:
                await self._speak(line)

        if not auto:
            return
        self._busy = True
        try:
            self._set_phase("thinking")
            self.status = "serving an incoming request…"
            reply = await self._guarded_turn(
                "You just CLAIMED these pending invocations from your inbox "
                f"(do not pull again): {json.dumps(auto)[:2000]}. For each, "
                "fulfil the capability — for negotiate_dinner use "
                "get_my_preferences and converge on a cuisine + drink — then "
                'call respond(invocation_id=<id>, status="succeeded", '
                "output=<result object>). Then tell the owner in one sentence "
                "what you agreed.",
                remember=False,
            )
            if reply:
                await self._on_agent_reply(reply, speak=True)
        finally:
            self._busy = False
            if not self._recording:
                self._set_phase("idle")
                self.status = "ready · press space to talk"

    async def _speak(self, text: str) -> None:
        """Speak a line via TTS without adding a transcript entry.
        Interruptible: see `_stop_speech` / action_ptt_toggle."""
        if not text:
            return
        self._set_phase("speaking")
        self._speaking = True
        self._stop_speech = False
        try:
            audio = await self.sarvam.synthesize(text)
            if not self._stop_speech:
                await play_wav_bytes_async(audio)
        except Exception:
            pass
        finally:
            self._speaking = False
            self._stop_speech = False
            if not self._recording and not self._busy:
                self._set_phase("idle")

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
        # Interrupt the agent mid-speech: stop playback so the user doesn't
        # have to sit through a long reply. The speaking turn then unwinds
        # and the NEXT space press records normally.
        if self._speaking:
            self._stop_speech = True
            stop_playback()
            self._speaking = False
            self._set_phase("idle")
            self.status = "stopped speaking · press space to talk"
            return

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

    async def _guarded_turn(self, prompt: str, *, remember: bool = True) -> str:
        """Run ONE agent turn, serialized against the inbox loop and
        time-bounded so a hung LLM/MCP call can't freeze the TUI forever.
        `remember=False` (inbox poll) keeps it out of the user dialogue."""
        async with self._turn_lock:
            try:
                return await asyncio.wait_for(
                    self.stack.run_turn(prompt, remember=remember),
                    timeout=TURN_TIMEOUT_S,
                )
            except asyncio.TimeoutError:
                return "Sorry, that took too long — let's try again."
            except Exception as e:
                # Surface the real failure into the transcript instead of
                # going silent — makes a broken turn diagnosable on camera.
                return f"⚠ couldn't finish that: {type(e).__name__}: {e}"

    async def _handle_utterance(self, wav: bytes) -> None:
        if not wav:
            self._set_phase("idle")
            self.status = "(nothing recorded — press space and speak)"
            return
        self._busy = True
        try:
            self._set_phase("transcribing")
            self.status = "transcribing…"
            text = await asyncio.wait_for(self.sarvam.transcribe(wav), timeout=40)
            if not text:
                self._set_phase("idle")
                self.status = "(heard silence — try again)"
                return
            assert self._transcript is not None
            self._transcript.add_line(
                f"[b]{self.stack.persona.display_name}:[/b] {text}"
            )
            self._set_phase("thinking")
            self.status = "thinking… (working on it)"
            # If a peer's message_owner is awaiting the owner's reply, give
            # the agent the context + ids so this spoken turn can complete
            # it (with confirmed_by_human=true) when the owner's words are
            # the answer.
            prompt = text
            if self._pending_owner_msgs:
                pend = "; ".join(
                    f'invocation {m["invocation_id"]} from {m["from"]}: "{m["message"]}"'
                    for m in self._pending_owner_msgs
                )
                prompt = (
                    f"[Pending human-in-the-loop messages awaiting your owner's "
                    f"reply: {pend}. If the owner's message below is their reply "
                    f"to one, send it: respond(invocation_id=<id>, "
                    f'status="succeeded", output={{"reply": <owner words>}}, '
                    f"confirmed_by_human=true).]\n\n{text}"
                )
            reply = await self._guarded_turn(prompt)
            # The owner has now had a chance to address them; drop so we
            # don't keep re-injecting stale ids.
            self._pending_owner_msgs.clear()
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
            self.status = "🔊 speaking — press space to stop"
            self._speaking = True
            self._stop_speech = False
            try:
                audio = await self.sarvam.synthesize(text)
                # If the user hit space during synthesis, don't start playing.
                if not self._stop_speech:
                    await play_wav_bytes_async(audio)
            finally:
                self._speaking = False
                self._stop_speech = False

    def notifier_sync(self, text: str) -> None:
        """The sync entry point passed to make_local_tools.

        The agent calls `update_owner_status(text)` from a tool, which
        runs ON the app's event loop — so `call_from_thread` would raise
        ("same thread"). Schedule directly when already on the loop, and
        only bounce via call_from_thread when invoked from another thread.
        """
        coro = self._on_agent_reply(text, speak=True)
        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            loop = None
        if loop is not None:
            loop.create_task(coro)
        else:
            self.call_from_thread(lambda: asyncio.create_task(coro))
