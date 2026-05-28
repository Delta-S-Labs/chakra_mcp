"""Type definitions mirroring the relay/app DTOs.

We use TypedDict throughout so the SDK has zero runtime overhead - at
runtime these are plain dicts; IDEs + mypy give you autocomplete and
type checking for free. Callers can still wrap them in pydantic models
or dataclasses if they want runtime validation; we don't impose that.
"""

from typing import Any, Literal, TypedDict

Visibility = Literal["private", "org", "network"]
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
    # Migration 0023: aggregates over the agent's un-hidden reviews.
    # ``avg_rating`` is ``None`` when ``review_count == 0``.
    avg_rating: float | None
    review_count: int


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
    # Migration 0022: owner-opt-in tier for public invocation.
    # True ⇒ any registered agent can call this capability without
    # a friendship/grant, bounded by `public_monthly_quota_per_agent`.
    public_invoke: bool
    # Per-invoker monthly cap (calendar month). Present only when
    # `public_invoke=True`.
    public_monthly_quota_per_agent: int | None


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


class FriendshipContext(TypedDict):
    """Trust context for a friendship - bundled in inbox responses."""

    id: str
    status: FriendshipStatus
    proposer_agent_id: str
    target_agent_id: str
    proposer_message: str | None
    response_message: str | None
    decided_at: str | None


class GrantContext(TypedDict):
    """Trust context for a grant - bundled in inbox responses."""

    id: str
    status: GrantStatus
    granter_agent_id: str
    grantee_agent_id: str
    capability_id: str
    capability_name: str
    capability_visibility: Visibility
    granted_at: str
    expires_at: str | None


class Invocation(TypedDict, total=False):
    """Inbox / audit-log row.

    The ``friendship_context`` and ``grant_context`` keys are populated
    only on inbox responses (``inbox.pull`` / ``inbox.serve``) - the
    relay just verified both before delivering, so handlers can trust
    these assertions without re-querying. They're absent on audit-log
    endpoints (``invocations.list / get``), where the live state of
    the friendship/grant may have drifted since the row was created.
    """

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
    # Capability semantics — `"autonomous"` or `"human_in_loop"`. Absent
    # on relays predating PR #80; absence is treated as autonomous by
    # `inbox.serve` for forward compatibility.
    semantics: str | None
    friendship_context: FriendshipContext  # only set on inbox responses
    grant_context: GrantContext  # only set on inbox responses


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
    # Migration 0022. Opt this capability into being callable without
    # a friendship/grant. When True, visibility must resolve to
    # "network" and `public_monthly_quota_per_agent` is required.
    public_invoke: bool
    public_monthly_quota_per_agent: int


class UpdateCapabilityRequest(TypedDict, total=False):
    description: str
    input_schema: dict[str, Any]
    output_schema: dict[str, Any]
    visibility: Visibility
    public_invoke: bool
    public_monthly_quota_per_agent: int


class ProposeFriendshipRequest(TypedDict, total=False):
    proposer_agent_id: str  # required
    target_agent_id: str  # required
    proposer_message: str | None


class CreateGrantRequest(TypedDict, total=False):
    granter_agent_id: str  # required
    grantee_agent_id: str  # required
    capability_id: str  # required
    expires_at: str | None


ReviewTier = Literal["friend", "public"]


class ReviewTag(TypedDict):
    capability_id: str
    capability_name: str


class Review(TypedDict):
    id: str
    reviewer: AgentSummary
    target: AgentSummary
    rating: int
    comment: str | None
    # Stamped at write-time. Won't drift if the friendship state
    # changes later.
    tier: ReviewTier
    tags: list[ReviewTag]
    hidden: bool
    created_at: str
    updated_at: str
    i_authored: bool


class ReviewDistribution(TypedDict):
    # JSON keys are stringly-typed bucket numbers, mirroring the relay.
    one: int
    two: int
    three: int
    four: int
    five: int


class ReviewSummary(TypedDict):
    # Mean rating over the *visible* set. ``None`` when ``count == 0``.
    average: float | None
    count: int
    distribution: dict[str, int]


class ReviewListResponse(TypedDict, total=False):
    reviews: list[Review]
    next_cursor: str | None
    summary: ReviewSummary


class WriteReviewRequest(TypedDict, total=False):
    reviewer_agent_id: str  # required
    rating: int  # required, 1-5
    comment: str | None
    tagged_capability_ids: list[str]  # required, length >= 1


class EligibleReviewer(TypedDict):
    reviewer_agent_id: str
    reviewer_display_name: str
    tagable_capability_ids: list[str]


class EligibilityResponse(TypedDict):
    eligible: list[EligibleReviewer]


class HandlerSucceeded(TypedDict):
    status: Literal["succeeded"]
    output: Any


class HandlerFailed(TypedDict, total=False):
    status: Literal["failed"]  # required
    error: str


HandlerResult = HandlerSucceeded | HandlerFailed
