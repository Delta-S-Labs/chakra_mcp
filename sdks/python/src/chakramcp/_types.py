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
    "pending",
    "in_progress",
    "succeeded",
    "failed",
    "rejected",
    "timeout",
    # Held until the target agent's owner approves it (owner consent).
    "awaiting_consent",
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


#: Who may call an agent: its friends, any agent, or nobody.
AcceptCalls = Literal["friends", "anyone", "nobody"]


class _AgentExtras(TypedDict, total=False):
    # Who may call this agent. Push agents always show ``"nobody"``.
    # Absent on relays without calls.
    accept_calls: AcceptCalls


class Agent(_AgentExtras):
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


#: How an agent gets work: the relay calls its endpoint (``"push"``), or
#: it pulls its inbox (``"pull"``).
AgentMode = Literal["push", "pull"]


class _NetworkAgentExtras(TypedDict, total=False):
    # Pulled its inbox in the last 5 minutes. Absent on relays before 0.4.0.
    online: bool
    # Who may call it; push agents show ``"nobody"``. Absent on relays
    # without calls.
    accept_calls: AcceptCalls


class NetworkAgent(_NetworkAgentExtras):
    """An agent in the network directory (``chakra.network()``)."""

    id: str
    account_id: str
    account_slug: str
    account_display_name: str
    slug: str
    display_name: str
    description: str
    mode: AgentMode
    tags: list[str]
    # The owning account is verified.
    verified: bool
    # Set on your own agents only; ``None`` on everyone else's.
    endpoint_url: str | None
    created_at: str
    updated_at: str
    is_mine: bool
    # The capabilities on it that you can see.
    capability_count: int
    # ``None`` when ``review_count == 0``.
    avg_rating: float | None
    review_count: int


class _NetworkPageExtras(TypedDict, total=False):
    # Pass back as ``cursor`` for the next page; absent on the last page.
    next_cursor: str


class NetworkPage(_NetworkPageExtras):
    """A page of the network directory, newest first."""

    agents: list[NetworkAgent]


class CapabilityConsent(TypedDict, total=False):
    """Owner consent settings: whether requests wait for the owner's
    approval. ``approvers`` only for those who manage the agent."""

    required: bool
    approvers: list[str]
    #: ``"once"``, ``"1h"``, ``"1d"``, ``"7d"`` or ``"30d"``.
    max_validity: str
    #: How long a held request waits: ``"<n>m"``, ``"<n>h"``, ``"<n>d"``.
    wait: str


class _CapabilityExtras(TypedDict, total=False):
    # `"autonomous"` or `"human_in_loop"`.
    semantics: str
    consent: CapabilityConsent


class Capability(_CapabilityExtras):
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


class ConsentTerms(TypedDict, total=False):
    """What a caller proposes when a capability asks its owner first. The
    owner may approve it, narrow it or deny it."""

    purpose: str
    #: ``"once"`` (default), ``"<n>h"`` (1-720) or ``"<n>d"`` (1-30).
    valid_for: str
    max_requests: int


class InvocationConsent(TypedDict):
    """What the owner approved, and the consent token proving it."""

    consent_request_id: str
    consent_id: str | None
    approved_at: str | None
    terms: ConsentTerms
    #: Check with :func:`verify_consent_token`. None until delivered.
    token: str | None


class HumanConfirmed(TypedDict):
    """A person confirmed the reply (``human_in_loop``)."""

    consent_request_id: str
    confirmed_at: str | None
    token: str | None


class SafetyFinding(TypedDict, total=False):
    """One thing message safety found, and what it did. Never the removed
    value: only its kind (``email``, ``phone``, ``harm_crime``…) and where
    it was (``path``, e.g. ``input.note``)."""

    kind: str
    #: ``redacted``, ``flagged`` or ``blocked``.
    action: str
    path: str
    #: The question that judged it, when the model did.
    question: str
    p: float


class Safety(TypedDict, total=False):
    """What message safety did to a call: ``request`` for the input,
    ``reply`` for the reply (shown to the caller once it's released)."""

    request: list[SafetyFinding]
    reply: list[SafetyFinding]


class _DraftedReplyExtras(TypedDict, total=False):
    # What message safety removed or flagged in the draft.
    safety: Safety


class DraftedReply(_DraftedReplyExtras):
    """A reply to a ``human_in_loop`` capability, kept as a draft until one
    of the agent's owners confirms it on the web (``approve_url``)."""

    status: Literal["awaiting_confirmation"]
    consent_request_id: str
    approve_url: str
    expires_at: str


class Notice(TypedDict, total=False):
    """Something an agent is told out of band of its inbox. A
    ``consent_requested`` notice means a call waits for your owner's
    approval: pass ``notice["consent_request"]["approve_url"]`` on. Others:
    ``reply_confirmed``, ``reply_rejected``, ``reply_expired``. Never
    includes the call's input."""

    id: str
    kind: str
    created_at: str
    consent_request: dict[str, Any]


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
    # Why the granter issued this grant, if they said.
    purpose: str | None
    i_granted: bool
    i_received: bool


class InvokeResponse(TypedDict, total=False):
    invocation_id: str
    status: InvocationStatus
    error: str | None
    # Set when the capability asks its owner first: held
    # (``awaiting_consent``) or covered by a standing consent (``pending``).
    consent_request_id: str
    # Why a rejected call was refused, when message safety refused it:
    # ``chk.policy.safety_blocked`` or ``chk.policy.compliance_denied``.
    code: str
    # What message safety removed, flagged or blocked in the request.
    safety: Safety


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
    # Why the granter issued this grant, if they said. May be missing
    # on relays / snapshots that predate grant purposes - read it with
    # ``.get("purpose")``.
    purpose: str | None


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
    # Which trust path authorised this invocation. ``"friend"`` =
    # rode a friendship + grant (friendship_context / grant_context
    # are populated on inbox responses). ``"public"`` = rode a
    # ``public_invoke=true`` capability (migration 0022); contexts
    # come back ``None``. Use this instead of inspecting grant_id or
    # the contexts directly — it removes the "null means public"
    # footgun for handlers. Absent on relays predating PR #145.
    tier: str
    # Capability semantics — `"autonomous"` or `"human_in_loop"` (a reply
    # is a draft until one of the agent's owners confirms it on the web).
    # Absence is treated as autonomous by `inbox.serve`.
    semantics: str | None
    # Set on inbox.pull responses (verified live by the relay) AND on
    # audit-log endpoints for rows queued on/after the trust-snapshot
    # migration (#146). On audit-log paths the contexts come from a
    # frozen JSONB snapshot captured at queue time — they reflect what
    # was true when the call was queued, not necessarily what's true
    # now. Absent on public-invoke rows (no friendship/grant existed)
    # and on legacy pre-snapshot rows.
    friendship_context: FriendshipContext
    grant_context: GrantContext
    # Owner consent: what the owner approved (a capability that asks
    # first), and that a person confirmed the reply (``human_in_loop``).
    consent: InvocationConsent
    human_confirmed: HumanConfirmed
    # What message safety removed, flagged or blocked in this call.
    safety: Safety
    # The call this invocation was made in, if any.
    call_id: str | None


# ─── Request bodies ──────────────────────────────────────


class InvokeRequest(TypedDict, total=False):
    """The body of ``invoke()``: exactly one of ``grant_id`` (a friendship
    and grant) or ``capability_id`` (a ``public_invoke`` capability)."""

    grant_id: str
    capability_id: str
    grantee_agent_id: str  # required: the agent making the request
    input: Any  # required
    # Proposed terms, for a capability that asks its owner first.
    consent_terms: ConsentTerms
    # Make the request in this call: the invoking agent must be a party to
    # it, and the capability the other party's.
    call_id: str


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
    # Who may call this agent. Push agents only take ``"nobody"``.
    accept_calls: AcceptCalls


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
    # `"human_in_loop"`: a reply is a draft until an owner confirms it.
    semantics: str
    # Owner consent: {required, approvers, max_validity, wait}.
    consent: dict[str, Any]


class UpdateCapabilityRequest(TypedDict, total=False):
    description: str
    input_schema: dict[str, Any]
    output_schema: dict[str, Any]
    visibility: Visibility
    public_invoke: bool
    public_monthly_quota_per_agent: int
    semantics: str
    consent: dict[str, Any]


class ProposeFriendshipRequest(TypedDict, total=False):
    proposer_agent_id: str  # required
    target_agent_id: str  # required
    proposer_message: str | None


class CreateGrantRequest(TypedDict, total=False):
    granter_agent_id: str  # required
    grantee_agent_id: str  # required
    capability_id: str  # required
    expires_at: str | None
    # Optional free-text reason the grantee needs access. Trimmed by the
    # relay (empty -> None); max 500 characters.
    purpose: str | None


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


# ─── Calls ───────────────────────────────────────────────

CallState = Literal["ringing", "active", "declined", "missed", "ended"]
#: ``"call"``: the relay keeps the call's memory. ``"own"``: each agent
#: keeps its own; the relay only stores the transcript.
CallMemoryMode = Literal["call", "own"]
CallEndedBy = Literal["caller", "callee", "owner", "idle", "gone"]
CallEntryKind = Literal[
    "invite", "message", "accepted", "declined", "ended", "missed", "invocation"
]


class _CallPartyExtras(TypedDict, total=False):
    display_name: str | None
    account_slug: str | None


class CallParty(_CallPartyExtras):
    """One side of a call. ``agent_id`` is ``None`` once the agent is
    deleted."""

    agent_id: str | None
    account_id: str


class CallMemoryFeatures(TypedDict):
    """Fixed at invite: whether the relay tags messages and answers
    ``calls.ask``."""

    tags: bool
    recall: bool


class _CallExtras(TypedDict, total=False):
    topic: str | None
    answered_at: str | None
    ended_at: str | None
    ended_by: CallEndedBy | None
    purge_at: str | None
    purged_at: str | None
    last_message_at: str | None


class Call(_CallExtras):
    """A call between two agents."""

    id: str
    state: CallState
    caller: CallParty
    callee: CallParty
    memory: CallMemoryMode
    memory_features: CallMemoryFeatures
    ttl_secs: int
    idle_secs: int
    created_at: str
    message_count: int


class _CallEntryInvocationExtras(TypedDict, total=False):
    capability: str | None
    status: str | None


class CallEntryInvocation(_CallEntryInvocationExtras):
    """The invocation an ``invocation`` entry stands for. Poll the
    invocation for its result."""

    id: str


class _CallEntryExtras(TypedDict, total=False):
    from_agent_id: str | None
    text: str | None
    data: Any
    # What message safety removed or flagged in this message.
    safety: list[SafetyFinding]
    invocation: CallEntryInvocation


class CallEntry(_CallEntryExtras):
    """One entry of a call's transcript, in ``seq`` order."""

    seq: int
    kind: CallEntryKind
    blocked: bool
    created_at: str


class _MemoryTagsExtras(TypedDict, total=False):
    # The ``seq`` of the earlier question the message answers, if any, and
    # how likely it answers it.
    resolves: int
    resolves_p: float


class MemoryTags(_MemoryTagsExtras):
    """How likely a message is each of these, and how much it matters:
    ``weight`` runs from 0 (chit-chat) to 3 (an agreed term or deadline)."""

    decision: float
    commitment: float
    open_question: float
    fact: float
    weight: float


class _MemoryInvocationExtras(TypedDict, total=False):
    # Sliced JSON, on finished invocations only.
    input: Any
    output: Any


class MemoryInvocation(_MemoryInvocationExtras):
    id: str
    capability: str
    status: str


# ``from`` is a keyword, so these use the functional syntax.
_MemoryItemExtras = TypedDict(
    "_MemoryItemExtras",
    {
        "from": str | None,
        "text": str | None,
        # The previous message, for a short pinned one ("Agreed.").
        "context": str | None,
        "tags": MemoryTags,
        "invocation": MemoryInvocation,
    },
    total=False,
)


class MemoryItem(_MemoryItemExtras):
    """A message (verbatim, possibly sliced) or an invocation in a call's
    memory. ``item["from"]`` is the sending agent's id."""

    seq: int
    at: str


class CallMemory(TypedDict):
    """A call's memory: ``recent`` messages verbatim, ``pinned``
    decisions, commitments and facts from earlier, and ``open``
    questions not yet answered."""

    pinned: list[MemoryItem]
    open: list[MemoryItem]
    recent: list[MemoryItem]


_RecallMessageExtras = TypedDict(
    "_RecallMessageExtras",
    {"from": str | None, "text": str | None},
    total=False,
)


class RecallMessage(_RecallMessageExtras):
    seq: int
    at: str
    #: How likely this message answers the question.
    p: float


class _CallRecallExtras(TypedDict, total=False):
    # How likely any of ``messages`` answers the question; ``None`` when
    # there was nothing to ask about.
    answered: float | None


class CallRecall(_CallRecallExtras):
    """What ``calls.ask`` found: the stored messages most likely to answer
    the question, best first. Read them and answer for yourself."""

    messages: list[RecallMessage]


class _CallFeedItemExtras(TypedDict, total=False):
    # In ``memory: "call"`` mode, on the newest message per call in a pull.
    memory: CallMemory


class CallFeedItem(_CallFeedItemExtras):
    """One item of an agent's calls feed (``calls.pull``): the call as it
    stands and the entry delivered to the agent."""

    call: Call
    entry: CallEntry


class _CallListExtras(TypedDict, total=False):
    next_cursor: str


class CallList(_CallListExtras):
    calls: list[Call]


class StartCallRequest(TypedDict, total=False):
    from_agent_id: str  # required: the calling agent, one of yours
    to_agent_id: str  # required
    # ``"own"`` (the default) or ``"call"``.
    memory: CallMemoryMode
    ttl_secs: int
    idle_secs: int
    topic: str
    # An opening message.
    message: str


class _CallReplyExtras(TypedDict, total=False):
    data: Any


class CallReply(_CallReplyExtras):
    """A reply from ``calls.serve``'s ``on_message``: text and optional
    structured data. Returning a plain string sends just the text."""

    text: str


class _InviteDecisionExtras(TypedDict, total=False):
    # When accepting: a first reply.
    message: str
    # When declining: why.
    reason: str


class InviteDecision(_InviteDecisionExtras):
    """What ``calls.serve``'s ``on_invite`` decides: ``{"accept": True}``
    (optionally with a first ``message``) or ``{"accept": False}``
    (optionally with a ``reason``)."""

    accept: bool
