"""Reserved capability templates.

These are capabilities the ChakraMCP protocol reserves a name for —
agents that publish the canonical name MUST conform to the canonical
schema. That lets peers discover and call a known capability without
per-agent integration code.

Templates are static dicts (not classes) because they're pure data —
just JSON Schema literals — and need to ship verbatim into both the
Python SDK and the published REST payload. ``CHAKRAMCP_TEMPLATES``
is the registry the rest of the SDK reads from. The
``capabilities.add_template()`` helpers in the async + sync clients
look up by id and POST the equivalent ``capabilities.create()`` body.

Adding a new template: append an entry below, bump the SDK minor
version, and document it in /docs/agents under "Reserved capability
templates" — that page is the canonical source for peers (incl. ones
not using this SDK) and explains why the names are reserved.
"""

from __future__ import annotations

from typing import Any, Final

# Each template is the body you'd pass to
# ``chakra.capabilities.create(agent_id, body)`` with name +
# description filled in. The user can override description at publish
# time; the schemas are locked.
MESSAGE_OWNER: Final[dict[str, Any]] = {
    "name": "message_owner",
    "description": (
        "Ping the human owner of this agent. Friend agents call this; "
        "the owner sees the message and explicitly answers, acks, "
        "ignores, or defers. Always human-in-the-loop."
    ),
    # Issue #69 PR 2: published capabilities now carry a `semantics`
    # field, and `message_owner` is the canonical human-in-the-loop
    # case. The relay's gate at POST /v1/invocations/{id}/result will
    # reject worker-posted replies on HITL invocations unless they
    # carry `confirmed_by_human: true`. The Python SDK's default reply
    # path leaves that flag off — autonomous handlers that try to
    # answer a `message_owner` invocation get a 409 with code
    # `chk.policy.requires_human_confirmation` and must route the
    # invocation to a human instead (PR 3 plumbs the `human_handler`
    # callback through `inbox.serve()`).
    "semantics": "human_in_loop",
    "input_schema": {
        "type": "object",
        "required": ["message"],
        "properties": {
            "message": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4000,
                "description": "Message body. Plain text. Be concise.",
            },
            "from_display_name": {
                "type": "string",
                "maxLength": 120,
                "description": "Who's pinging. Falls back to calling agent's display_name.",
            },
            "urgency": {
                "type": "string",
                "enum": ["low", "normal", "high"],
                "default": "normal",
                "description": "low = batch in next digest; normal = surface when owner next interacts; high = alert immediately.",
            },
            "expects_reply": {
                "type": "boolean",
                "default": True,
                "description": "If false, message is informational and an ack is enough.",
            },
            "reply_by": {
                "type": "string",
                "format": "date-time",
                "description": "Optional soft deadline. After this, handler may auto-defer.",
            },
        },
    },
    "output_schema": {
        "type": "object",
        "required": ["status"],
        "properties": {
            "status": {
                "type": "string",
                "enum": ["replied", "acknowledged", "ignored", "deferred"],
            },
            "reply": {
                "type": "string",
                "maxLength": 8000,
                "description": "Owner's reply text. Present only when status='replied'.",
            },
            "responded_at": {
                "type": "string",
                "format": "date-time",
            },
            "defer_until": {
                "type": "string",
                "format": "date-time",
                "description": "Present only when status='deferred'.",
            },
        },
    },
    "visibility": "network",
}


# Registry — id → body. SDK clients call ``CHAKRAMCP_TEMPLATES[id]``.
# Order doesn't matter; iteration is alphabetical when listed.
CHAKRAMCP_TEMPLATES: Final[dict[str, dict[str, Any]]] = {
    "message_owner": MESSAGE_OWNER,
}


def template_names() -> list[str]:
    """List of reserved template ids. Stable order (sorted)."""
    return sorted(CHAKRAMCP_TEMPLATES.keys())


def get_template(template_id: str) -> dict[str, Any]:
    """Look up a template by id. Raises ``KeyError`` if unknown.

    Returned dict is a *deep-copy-safe reference* — callers should
    not mutate. If you need a mutated version (e.g. to override
    description), copy it explicitly:

        from copy import deepcopy
        body = deepcopy(get_template("message_owner"))
        body["description"] = "Custom wording for my agent"
    """
    if template_id not in CHAKRAMCP_TEMPLATES:
        known = ", ".join(template_names()) or "(none)"
        raise KeyError(
            f"unknown template id: {template_id!r}. Known: {known}"
        )
    return CHAKRAMCP_TEMPLATES[template_id]
