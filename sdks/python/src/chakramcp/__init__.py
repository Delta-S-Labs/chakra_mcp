"""ChakraMCP - Python SDK for the relay.

Two clients with the same surface:

* :class:`ChakraMCP` - synchronous, for scripts and notebooks.
* :class:`AsyncChakraMCP` - asyncio, for agent runtimes and webhooks.

Both share the same sub-clients (``.agents``, ``.friendships``,
``.grants``, ``.invocations``, ``.inbox``) and the same convenience
helpers (``invoke_and_wait``, ``inbox.serve``).
"""

__version__ = "0.3.5"

from ._async import AsyncChakraMCP
from ._errors import ChakraMCPError, QuotaExhaustedError
from ._sync import ChakraMCP
from ._templates import (
    CHAKRAMCP_TEMPLATES,
    MESSAGE_OWNER,
    get_template,
    template_names,
)
from ._types import (
    TERMINAL_STATUSES,
    Agent,
    AgentSummary,
    Capability,
    CreateAgentRequest,
    CreateCapabilityRequest,
    CreateGrantRequest,
    EligibilityResponse,
    EligibleReviewer,
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
    Review,
    ReviewDistribution,
    ReviewListResponse,
    ReviewSummary,
    ReviewTag,
    ReviewTier,
    UpdateAgentRequest,
    UpdateCapabilityRequest,
    User,
    Visibility,
    WriteReviewRequest,
)

__all__ = [
    "AsyncChakraMCP",
    "ChakraMCP",
    "ChakraMCPError",
    "QuotaExhaustedError",
    "CHAKRAMCP_TEMPLATES",
    "MESSAGE_OWNER",
    "get_template",
    "template_names",
    "TERMINAL_STATUSES",
    "Agent",
    "AgentSummary",
    "Capability",
    "CreateAgentRequest",
    "CreateCapabilityRequest",
    "UpdateCapabilityRequest",
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
    "Review",
    "ReviewDistribution",
    "ReviewListResponse",
    "ReviewSummary",
    "ReviewTag",
    "ReviewTier",
    "EligibilityResponse",
    "EligibleReviewer",
    "WriteReviewRequest",
    "UpdateAgentRequest",
    "User",
    "Visibility",
    "__version__",
]
