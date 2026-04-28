"""ChakraMCP - Python SDK for the relay.

Two clients with the same surface:

* :class:`ChakraMCP` - synchronous, for scripts and notebooks.
* :class:`AsyncChakraMCP` - asyncio, for agent runtimes and webhooks.

Both share the same sub-clients (``.agents``, ``.friendships``,
``.grants``, ``.invocations``, ``.inbox``) and the same convenience
helpers (``invoke_and_wait``, ``inbox.serve``).
"""

__version__ = "0.1.0"

from ._async import AsyncChakraMCP
from ._errors import ChakraMCPError
from ._sync import ChakraMCP
from ._types import (
    TERMINAL_STATUSES,
    Agent,
    AgentSummary,
    Capability,
    CreateAgentRequest,
    CreateCapabilityRequest,
    CreateGrantRequest,
    Friendship,
    FriendshipContext,
    FriendshipStatus,
    Grant,
    GrantContext,
    GrantStatus,
    HandlerFailed,
    HandlerResult,
    HandlerSucceeded,
    Invocation,
    InvocationStatus,
    InvokeResponse,
    Membership,
    MeResponse,
    ProposeFriendshipRequest,
    ResultStatus,
    UpdateAgentRequest,
    User,
    Visibility,
)

__all__ = [
    "AsyncChakraMCP",
    "ChakraMCP",
    "ChakraMCPError",
    "TERMINAL_STATUSES",
    "Agent",
    "AgentSummary",
    "Capability",
    "CreateAgentRequest",
    "CreateCapabilityRequest",
    "CreateGrantRequest",
    "Friendship",
    "FriendshipContext",
    "FriendshipStatus",
    "Grant",
    "GrantContext",
    "GrantStatus",
    "HandlerFailed",
    "HandlerResult",
    "HandlerSucceeded",
    "Invocation",
    "InvocationStatus",
    "InvokeResponse",
    "Membership",
    "MeResponse",
    "ProposeFriendshipRequest",
    "ResultStatus",
    "UpdateAgentRequest",
    "User",
    "Visibility",
    "__version__",
]
