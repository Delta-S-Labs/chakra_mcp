"""TracingProcessor → Logs tab feed.

The OpenAI Agents SDK emits a span for *every* unit of work — each LLM
generation, each function-tool call, each MCP tool call, each MCP
list-tools poke. We register a `TracingProcessor` that gets called
synchronously on `on_span_start` / `on_span_end`, route the events
through an asyncio.Queue, and the Textual app drains the queue into
an expandable tree.

We carry the start-time and metadata on the way in so the end event
can show duration + a friendly classification (LLM / MCP / function /
…) without re-introspecting the span type.
"""

from __future__ import annotations

import asyncio
import time
from dataclasses import dataclass, field
from typing import Any

from agents.tracing import Span, Trace, TracingProcessor


@dataclass
class LogEvent:
    """One row in the Logs tab tree."""

    span_id: str
    parent_id: str | None
    kind: str               # "llm" | "mcp" | "function" | "agent" | "generic"
    label: str              # one-line header rendered in the tree
    started_at: float
    ended_at: float | None = None
    details: dict[str, Any] = field(default_factory=dict)

    @property
    def duration_ms(self) -> int | None:
        if self.ended_at is None:
            return None
        return int((self.ended_at - self.started_at) * 1000)


def _jsonable(v: Any) -> Any:
    """Coerce SDK values (pydantic models, nested objects) into plain
    JSON-serializable structures so the Logs tab can pretty-print the
    *real* content instead of an opaque `<Object ...>` repr.
    """
    if v is None or isinstance(v, (str, int, float, bool)):
        return v
    # pydantic v2 models (Response, Usage, message items, …)
    if hasattr(v, "model_dump"):
        try:
            return _jsonable(v.model_dump(exclude_none=True))
        except Exception:
            return str(v)
    if isinstance(v, dict):
        return {k: _jsonable(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [_jsonable(x) for x in v]
    # dataclass-ish / arbitrary objects → best-effort dict, else str.
    if hasattr(v, "__dict__") and vars(v):
        return {k: _jsonable(x) for k, x in vars(v).items() if not k.startswith("_")}
    return str(v)


def _response_output_text(resp: Any) -> str | None:
    """Best-effort extraction of the assistant's visible text from a
    Responses-API Response object, so the Logs tab shows what the model
    actually said without the operator hunting through the full dump."""
    if resp is None:
        return None
    # SDK convenience attribute, when present.
    txt = getattr(resp, "output_text", None)
    if isinstance(txt, str) and txt.strip():
        return txt
    # Otherwise walk output items → content → text.
    out = getattr(resp, "output", None)
    if not out:
        return None
    chunks: list[str] = []
    for item in out:
        content = getattr(item, "content", None) or []
        for c in content:
            t = getattr(c, "text", None)
            if isinstance(t, str):
                chunks.append(t)
    joined = "\n".join(chunks).strip()
    return joined or None


def _classify(span: Span[Any]) -> tuple[str, str, dict[str, Any]]:
    """Map a span to (kind, label, details).

    Span class names verified against `agents.tracing.span_data` in the
    installed SDK — `FunctionSpanData.mcp_data` is the field that
    distinguishes an MCP-backed tool call from a local function tool,
    which is why we check that one *first*.
    """
    sd_ = span.span_data
    cls = type(sd_).__name__

    # ---- Function tool — local OR MCP-backed --------------------------
    # FunctionSpanData carries an `mcp_data` attribute populated only
    # when the tool call routed through an MCP server. We use that to
    # render MCP calls as "<server> · <tool>" and local function tools
    # as "tool · <name>".
    if cls == "FunctionSpanData":
        name = getattr(sd_, "name", "?")
        mcp_data = getattr(sd_, "mcp_data", None)
        if mcp_data:
            # mcp_data is typically a dict with server / server_name.
            server = (
                mcp_data.get("server")
                or mcp_data.get("server_name")
                or "mcp"
                if isinstance(mcp_data, dict)
                else "mcp"
            )
            return (
                "mcp",
                f"{server} · {name}",
                {
                    "input": _jsonable(getattr(sd_, "input", None)),
                    "output": _jsonable(getattr(sd_, "output", None)),
                },
            )
        return (
            "function",
            f"tool · {name}",
            {
                "input": _jsonable(getattr(sd_, "input", None)),
                "output": _jsonable(getattr(sd_, "output", None)),
            },
        )

    # ---- MCP list-tools poke ------------------------------------------
    if cls == "MCPListToolsSpanData":
        server = getattr(sd_, "server", "mcp")
        return (
            "mcp",
            f"{server} · list_tools",
            {"result": _jsonable(getattr(sd_, "result", None))},
        )

    # ---- LLM hop (Responses API path — what the SDK actually emits) ---
    if cls == "ResponseSpanData":
        resp = getattr(sd_, "response", None)
        return (
            "llm",
            "LLM · response",
            {
                # Lead with the human-readable answer, then the structured bits.
                "output_text": _response_output_text(resp),
                "input": _jsonable(getattr(sd_, "input", None)),
                "usage": _jsonable(getattr(resp, "usage", None)),
                "response": _jsonable(resp),
            },
        )

    # ---- Legacy generation span (in case the SDK falls back to it) ----
    if cls == "GenerationSpanData":
        model = getattr(sd_, "model", None) or "?"
        return (
            "llm",
            f"LLM · {model}",
            {
                "model": model,
                "input": _jsonable(getattr(sd_, "input", None)),
                "output": _jsonable(getattr(sd_, "output", None)),
                "usage": _jsonable(getattr(sd_, "usage", None)),
            },
        )

    # ---- Agent run frame ----------------------------------------------
    if cls == "AgentSpanData":
        return (
            "agent",
            f"agent · {getattr(sd_, 'name', 'agent')}",
            {
                "tools": _jsonable(getattr(sd_, "tools", None)),
            },
        )

    # ---- Catch-all -----------------------------------------------------
    return ("generic", cls.replace("SpanData", ""), {"raw": repr(sd_)[:200]})


class QueueProcessor(TracingProcessor):
    """Forwards span lifecycle events to an asyncio.Queue.

    The Textual app calls `attach(queue)` once at startup and drains the
    queue forever. Events are simple dicts: `{"type": "span_start" |
    "span_end" | "trace_start" | "trace_end", "event": <LogEvent>?}`.
    """

    def __init__(self) -> None:
        self._queue: asyncio.Queue[dict[str, Any]] | None = None
        self._loop: asyncio.AbstractEventLoop | None = None
        self._open: dict[str, LogEvent] = {}

    def attach(
        self, queue: asyncio.Queue[dict[str, Any]], loop: asyncio.AbstractEventLoop
    ) -> None:
        self._queue = queue
        self._loop = loop

    # ---- TracingProcessor interface ------------------------------------

    def on_trace_start(self, trace: Trace) -> None:
        self._emit({"type": "trace_start", "name": getattr(trace, "name", "trace")})

    def on_trace_end(self, trace: Trace) -> None:
        self._emit({"type": "trace_end", "name": getattr(trace, "name", "trace")})

    def on_span_start(self, span: Span[Any]) -> None:
        kind, label, details = _classify(span)
        ev = LogEvent(
            span_id=span.span_id,
            parent_id=getattr(span, "parent_id", None),
            kind=kind,
            label=label,
            started_at=time.monotonic(),
            details=details,
        )
        self._open[span.span_id] = ev
        self._emit({"type": "span_start", "event": ev})

    def on_span_end(self, span: Span[Any]) -> None:
        ev = self._open.pop(span.span_id, None)
        if ev is None:
            # Span we never saw open (shouldn't happen, but don't crash).
            kind, label, details = _classify(span)
            ev = LogEvent(
                span_id=span.span_id,
                parent_id=getattr(span, "parent_id", None),
                kind=kind,
                label=label,
                started_at=time.monotonic(),
                details=details,
            )
        # Re-classify on end so we pick up final input/output values
        # (some span data fields are only populated by the time it closes).
        _, label, details = _classify(span)
        ev.label = label
        ev.details = details
        ev.ended_at = time.monotonic()
        self._emit({"type": "span_end", "event": ev})

    def shutdown(self) -> None:
        return

    def force_flush(self) -> None:
        return

    # ---- internal -----------------------------------------------------

    def _emit(self, msg: dict[str, Any]) -> None:
        if self._queue is None or self._loop is None:
            return
        # TracingProcessor is called synchronously from the agent's
        # thread; bounce into the Textual loop via call_soon_threadsafe.
        self._loop.call_soon_threadsafe(self._queue.put_nowait, msg)
