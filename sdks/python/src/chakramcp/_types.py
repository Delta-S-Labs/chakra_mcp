"""Type definitions mirroring the relay/app DTOs.

We use TypedDict throughout so the SDK has zero runtime overhead — at
runtime these are plain dicts; IDEs + mypy give you autocomplete and
type checking for free. Callers can still wrap them in pydantic models
or dataclasses if they want runtime validation; we don't impose that.
"""

from typing import Any, Literal, TypedDict

Visibility = Literal["private", "network"]
FriendshipStatus = Literal[
    "proposed", "accepted", "rejected", "cancelled", "countered"
]
GrantStatus = Literal["active", "revoked", "expired"]
InvocationStatus = Literal[
    "pending", "in_progress", "succeeded", "failed", "rejected", "timeout"
]
ResultStatus = Literal["succeeded", "failed"]

TERMINAL_STATUSES: frozenset[InvocationStatus] = frozenset(
    {"succeeded", "failed", "rejected", "timeout"}
)


class User(TypedDict):
    id: str
    email: str
    display_name: str
    avatar_url: str | None
    is_admin: bool


class Membership(TypedDict):
    account_id: str
    slug: str
    display_name: str
    account_type: Literal["individual", "organization"]
    role: Literal["owner", "admin", "member"]


class MeResponse(TypedDict):
    user: User
    memberships: list[Membership]
    survey_required: bool


class Agent(TypedDict):
    id: str
    account_id: str
    account_slug: str
    account_display_name: str
    slug: str
    display_name: str
    description: str
    visibility: Visibility
    endpoint_url: str | None
    created_at: str
    updated_at: str
    is_mine: bool
    capability_count: int


class Capability(TypedDict):
    id: str
    agent_id: str
    name: str
    description: str
    input_schema: dict[str, Any]
    output_schema: dict[str, Any]
    visibility: Visibility
    created_at: str
    updated_at: str


class AgentSummary(TypedDict):
    id: str
    slug: str
    display_name: str
    account_id: str
    account_slug: str
    account_display_name: str


class Friendship(TypedDict):
    id: str
    status: FriendshipStatus
    proposer: AgentSummary
    target: AgentSummary
    proposer_message: str | None
    response_message: str | None
    counter_of_id: str | None
    created_at: str
    updated_at: str
    decided_at: str | None
    i_proposed: bool
    i_received: bool


class Grant(TypedDict):
    id: str
    status: GrantStatus
    granter: AgentSummary
    grantee: AgentSummary
    capability_id: str
    capability_name: str
    capability_visibility: Visibility
    granted_at: str
    expires_at: str | None
    revoked_at: str | None
    revoke_reason: str | None
    i_granted: bool
    i_received: bool


class InvokeResponse(TypedDict):
    invocation_id: str
    status: InvocationStatus
    error: str | None


class Invocation(TypedDict):
    id: str
    grant_id: str | None
    granter_agent_id: str | None
    granter_display_name: str | None
    grantee_agent_id: str | None
    grantee_display_name: str | None
    capability_id: str | None
    capability_name: str
    status: InvocationStatus
    elapsed_ms: int
    error_message: str | None
    input_preview: Any | None
    output_preview: Any | None
    created_at: str
    claimed_at: str | None
    i_served: bool
    i_invoked: bool


# ─── Request bodies ──────────────────────────────────────


class CreateAgentRequest(TypedDict, total=False):
    account_id: str  # required
    slug: str  # required
    display_name: str  # required
    description: str
    visibility: Visibility
    endpoint_url: str | None


class UpdateAgentRequest(TypedDict, total=False):
    display_name: str
    description: str
    visibility: Visibility
    endpoint_url: str | None


class CreateCapabilityRequest(TypedDict, total=False):
    name: str  # required
    description: str
    input_schema: dict[str, Any]
    output_schema: dict[str, Any]
    visibility: Visibility


class ProposeFriendshipRequest(TypedDict, total=False):
    proposer_agent_id: str  # required
    target_agent_id: str  # required
    proposer_message: str | None


class CreateGrantRequest(TypedDict, total=False):
    granter_agent_id: str  # required
    grantee_agent_id: str  # required
    capability_id: str  # required
    expires_at: str | None


class HandlerSucceeded(TypedDict):
    status: Literal["succeeded"]
    output: Any


class HandlerFailed(TypedDict, total=False):
    status: Literal["failed"]  # required
    error: str


HandlerResult = HandlerSucceeded | HandlerFailed
