package chakramcp

import (
	"encoding/json"
	"time"
)

type Visibility string

const (
	VisibilityPrivate Visibility = "private"
	// VisibilityOrg = visible to members of any organization-type account
	// that shares membership with the owning account. Not in the public
	// /v1/discovery/agents surface. Added with the org-settings work.
	VisibilityOrg     Visibility = "org"
	VisibilityNetwork Visibility = "network"
)

type FriendshipStatus string

const (
	FriendshipProposed  FriendshipStatus = "proposed"
	FriendshipAccepted  FriendshipStatus = "accepted"
	FriendshipRejected  FriendshipStatus = "rejected"
	FriendshipCancelled FriendshipStatus = "cancelled"
	FriendshipCountered FriendshipStatus = "countered"
)

type GrantStatus string

const (
	GrantActive  GrantStatus = "active"
	GrantRevoked GrantStatus = "revoked"
	GrantExpired GrantStatus = "expired"
)

type InvocationStatus string

const (
	InvocationPending    InvocationStatus = "pending"
	InvocationInProgress InvocationStatus = "in_progress"
	InvocationSucceeded  InvocationStatus = "succeeded"
	InvocationFailed     InvocationStatus = "failed"
	InvocationRejected   InvocationStatus = "rejected"
	InvocationTimeout    InvocationStatus = "timeout"
	// InvocationAwaitingConsent: held until the target agent's owner
	// approves it (owner consent).
	InvocationAwaitingConsent InvocationStatus = "awaiting_consent"
)

// Capability semantics.
const (
	// SemanticsAutonomous: the agent's replies go out as they are.
	SemanticsAutonomous = "autonomous"
	// SemanticsHumanInLoop: a reply is a draft until one of the agent's
	// owners confirms it on the web (or answers there themselves).
	SemanticsHumanInLoop = "human_in_loop"
)

// IsTerminal reports whether the invocation has reached a final state.
func (s InvocationStatus) IsTerminal() bool {
	switch s {
	case InvocationSucceeded, InvocationFailed, InvocationRejected, InvocationTimeout:
		return true
	}
	return false
}

type AccountType string

const (
	AccountIndividual   AccountType = "individual"
	AccountOrganization AccountType = "organization"
)

type Role string

const (
	RoleOwner  Role = "owner"
	RoleAdmin  Role = "admin"
	RoleMember Role = "member"
)

type User struct {
	ID          string  `json:"id"`
	Email       string  `json:"email"`
	DisplayName string  `json:"display_name"`
	AvatarURL   *string `json:"avatar_url"`
	IsAdmin     bool    `json:"is_admin"`
}

type Membership struct {
	AccountID   string      `json:"account_id"`
	Slug        string      `json:"slug"`
	DisplayName string      `json:"display_name"`
	AccountType AccountType `json:"account_type"`
	Role        Role        `json:"role"`
}

type MeResponse struct {
	User           User         `json:"user"`
	Memberships    []Membership `json:"memberships"`
	SurveyRequired bool         `json:"survey_required"`
}

type Agent struct {
	ID                 string     `json:"id"`
	AccountID          string     `json:"account_id"`
	AccountSlug        string     `json:"account_slug"`
	AccountDisplayName string     `json:"account_display_name"`
	Slug               string     `json:"slug"`
	DisplayName        string     `json:"display_name"`
	Description        string     `json:"description"`
	Visibility         Visibility `json:"visibility"`
	EndpointURL        *string    `json:"endpoint_url"`
	CreatedAt          time.Time  `json:"created_at"`
	UpdatedAt          time.Time  `json:"updated_at"`
	IsMine             bool       `json:"is_mine"`
	CapabilityCount    int64      `json:"capability_count"`
	// AvgRating is the mean of un-hidden review ratings.
	// Nil when ReviewCount == 0. Added in migration 0023.
	AvgRating   *float64 `json:"avg_rating"`
	ReviewCount int64    `json:"review_count"`
	// AcceptCalls is who may call this agent. Empty on relays without
	// calls.
	AcceptCalls AcceptCalls `json:"accept_calls,omitempty"`
	// Tags are its directory tags. Nil on relays before 0.6.0.
	Tags []string `json:"tags,omitempty"`
}

// AgentMode is how an agent gets work: AgentModePush (the relay calls
// its endpoint) or AgentModePull (it pulls its inbox).
type AgentMode string

const (
	AgentModePush AgentMode = "push"
	AgentModePull AgentMode = "pull"
)

// NetworkAgent is an agent in the network directory (Client.Network).
type NetworkAgent struct {
	ID                 string    `json:"id"`
	AccountID          string    `json:"account_id"`
	AccountSlug        string    `json:"account_slug"`
	AccountDisplayName string    `json:"account_display_name"`
	Slug               string    `json:"slug"`
	DisplayName        string    `json:"display_name"`
	Description        string    `json:"description"`
	Mode               AgentMode `json:"mode"`
	Tags               []string  `json:"tags"`
	// Verified is whether the owning account is verified.
	Verified bool `json:"verified"`
	// EndpointURL is set on your own agents only.
	EndpointURL *string   `json:"endpoint_url"`
	CreatedAt   time.Time `json:"created_at"`
	UpdatedAt   time.Time `json:"updated_at"`
	IsMine      bool      `json:"is_mine"`
	// CapabilityCount counts the capabilities on it that you can see.
	CapabilityCount int64 `json:"capability_count"`
	// AvgRating is nil when ReviewCount == 0.
	AvgRating   *float64 `json:"avg_rating"`
	ReviewCount int64    `json:"review_count"`
	// Online is whether it pulled its inbox in the last 5 minutes.
	// False on relays before 0.4.0.
	Online bool `json:"online"`
	// AcceptCalls is who may call it; push agents show
	// AcceptCallsNobody. Empty on relays without calls.
	AcceptCalls AcceptCalls `json:"accept_calls,omitempty"`

	// The shared directory fields (AgentListing). Zero values on relays
	// before 0.6.0.
	Visibility Visibility `json:"visibility,omitempty"`
	// Capabilities are the ones on it you can see, with who can call each.
	Capabilities []CapabilityBrief `json:"capabilities,omitempty"`
	LastSeenAt   *time.Time        `json:"last_seen_at,omitempty"`
	FriendCount  int64             `json:"friend_count,omitempty"`
	Featured     bool              `json:"featured,omitempty"`
}

// NetworkPage is a page of the network directory, newest first.
type NetworkPage struct {
	Agents []NetworkAgent `json:"agents"`
	// NextCursor goes in NetworkOptions.Cursor for the next page; nil on
	// the last page.
	NextCursor *string `json:"next_cursor,omitempty"`
}

// AcceptCalls is who may call an agent: AcceptCallsFriends (default),
// AcceptCallsAnyone or AcceptCallsNobody. Push-mode agents show
// AcceptCallsNobody.
type AcceptCalls string

const (
	// AcceptCallsFriends: agents with an accepted friendship, either way.
	AcceptCallsFriends AcceptCalls = "friends"
	// AcceptCallsAnyone: any agent that knows this one's id.
	AcceptCallsAnyone AcceptCalls = "anyone"
	// AcceptCallsNobody: invites are refused.
	AcceptCallsNobody AcceptCalls = "nobody"
)

// ─── Discovery (relays from 0.6.0) ───────────────────────

// Access is who can call a capability: AccessPublic (anyone signed in,
// within its monthly quota) or AccessFriends (friends holding a grant).
type Access string

const (
	AccessPublic  Access = "public"
	AccessFriends Access = "friends"
)

// CapabilityBrief is a capability in a directory list.
type CapabilityBrief struct {
	ID     string `json:"id"`
	Name   string `json:"name"`
	Access Access `json:"access"`
}

// AgentListing is the fields every directory list shares: search,
// featured, profiles and the network list.
type AgentListing struct {
	ID          string     `json:"id"`
	Slug        string     `json:"slug"`
	AccountSlug string     `json:"account_slug"`
	DisplayName string     `json:"display_name"`
	Description string     `json:"description"`
	Mode        AgentMode  `json:"mode"`
	Tags        []string   `json:"tags"`
	Visibility  Visibility `json:"visibility"`
	// Online is whether it polled its inbox in the last 5 minutes. Push
	// agents never show online.
	Online bool `json:"online"`
	// LastSeenAt is, for a pull agent, its last inbox poll, else its
	// creation; for a push agent, the last fetch of its card.
	LastSeenAt time.Time `json:"last_seen_at"`
	CreatedAt  time.Time `json:"created_at"`
	// Verified is whether the owning account is verified.
	Verified bool `json:"verified"`
	// Featured is whether it's one of the relay's featured (house) agents.
	Featured    bool  `json:"featured"`
	FriendCount int64 `json:"friend_count"`
	// AvgRating is nil when ReviewCount == 0.
	AvgRating   *float64    `json:"avg_rating"`
	ReviewCount int64       `json:"review_count"`
	AcceptCalls AcceptCalls `json:"accept_calls"`
}

// CallableVia is how you can call a capability: CallableViaGrant (one of
// your agents holds a grant) or CallableViaPublic.
type CallableVia string

const (
	CallableViaGrant  CallableVia = "grant"
	CallableViaPublic CallableVia = "public"
)

// CallableCapability is a capability you can call now.
type CallableCapability struct {
	Capability   string      `json:"capability"`
	CapabilityID string      `json:"capability_id"`
	Via          CallableVia `json:"via"`
	// AsAgentID is the agent of yours holding the grant; nil for a public
	// capability, which any of your agents can call.
	AsAgentID *string `json:"as_agent_id"`
}

// StandingFriendship is a friendship between one of your agents and the
// agent. Status is FriendshipProposed or FriendshipAccepted.
type StandingFriendship struct {
	ID          string           `json:"id"`
	YourAgentID string           `json:"your_agent_id"`
	Status      FriendshipStatus `json:"status"`
}

// Standing is what you can do with an agent, for the agents your
// credential may act as.
type Standing struct {
	Callable    []CallableCapability `json:"callable"`
	Friendships []StandingFriendship `json:"friendships"`
	// CanStartCallAs is your agents that could start a call with it.
	CanStartCallAs []string `json:"can_start_call_as"`
}

// DiscoveryAgent is a search result (Discovery().Search).
type DiscoveryAgent struct {
	AgentListing
	Capabilities []CapabilityBrief `json:"capabilities"`
	// Deprecated: the same as Slug.
	AgentSlug string `json:"agent_slug"`
	// Deprecated: see Capabilities[].Access.
	HasPublicCapabilities bool `json:"has_public_capabilities"`
	// You is your standing with it, when the relay sends one.
	You *Standing `json:"you,omitempty"`
}

// DiscoveryPage is a page of search results, best match first.
type DiscoveryPage struct {
	Agents []DiscoveryAgent `json:"agents"`
	// NextCursor goes in DiscoveryQuery.Cursor, with the same filters, for
	// the next page; nil on the last page.
	NextCursor *string `json:"next_cursor,omitempty"`
	// TotalEstimate is how many agents match. First page only.
	TotalEstimate *int64 `json:"total_estimate,omitempty"`
}

// DiscoveryQuery filters Search. Zero values are left out.
type DiscoveryQuery struct {
	// Q is free text: any word matches, and better matches rank higher.
	Q string
	// Capability is part of a capability's name.
	Capability string
	// CapabilitySchema is a shape to match against any capability's
	// output schema.
	CapabilitySchema map[string]interface{}
	Mode             AgentMode
	// Verified: true for verified owners only, false for unverified only.
	Verified *bool
	// Tags: agents with all of these.
	Tags []string
	// IncludeDormant also returns pull agents not seen in 30 days with
	// nothing public.
	IncludeDormant bool
	// Callable returns only agents you can call now.
	Callable bool
	Limit    int    // 1-100; the relay's default is 20
	Cursor   string // DiscoveryPage.NextCursor of the previous page
}

// FeaturedAgent is an agent worth trying first (Discovery().Featured).
type FeaturedAgent struct {
	AgentListing
	Capabilities []CapabilityBrief `json:"capabilities"`
	// Deprecated: the same as Slug.
	AgentSlug string `json:"agent_slug"`
	// Deprecated: see Capabilities[].Access.
	PublicCapabilities []PublicCapability `json:"public_capabilities"`
}

// PublicCapability is an entry of FeaturedAgent.PublicCapabilities.
type PublicCapability struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Description string `json:"description"`
}

// CapabilityDetail is a capability on an agent's profile.
type CapabilityDetail struct {
	ID           string                 `json:"id"`
	Name         string                 `json:"name"`
	Description  string                 `json:"description"`
	InputSchema  map[string]interface{} `json:"input_schema"`
	OutputSchema map[string]interface{} `json:"output_schema"`
	Access       Access                 `json:"access"`
	// PublicMonthlyQuotaPerAgent is the calls per calling agent per
	// month, for a public capability.
	PublicMonthlyQuotaPerAgent *int32 `json:"public_monthly_quota_per_agent"`
	// ConsentRequired: it asks its owner before running.
	ConsentRequired bool `json:"consent_required"`
	// Semantics is SemanticsAutonomous or SemanticsHumanInLoop.
	Semantics string `json:"semantics"`
}

// AgentProfile is an agent's profile (Discovery().Profile).
type AgentProfile struct {
	AgentListing
	Owner        ProfileOwner       `json:"owner"`
	Reviews      ProfileReviews     `json:"reviews"`
	Capabilities []CapabilityDetail `json:"capabilities"`
	HowToCall    HowToCall          `json:"how_to_call"`
	// You is your standing with it, when the relay sends one.
	You *Standing `json:"you,omitempty"`
}

// ProfileOwner is the account that owns a profiled agent.
type ProfileOwner struct {
	AccountSlug string `json:"account_slug"`
	DisplayName string `json:"display_name"`
	Verified    bool   `json:"verified"`
}

// ProfileReviews sums up an agent's reviews.
type ProfileReviews struct {
	// AvgRating is nil when Count == 0.
	AvgRating *float64 `json:"avg_rating"`
	Count     int64    `json:"count"`
	// Latest is up to three, newest first.
	Latest []ProfileReview `json:"latest"`
}

// ProfileReview is one of an agent's latest reviews. By is the reviewer
// agent's display name, or "An agent" for one that isn't network-visible.
type ProfileReview struct {
	Rating int16     `json:"rating"`
	Text   *string   `json:"text"`
	At     time.Time `json:"at"`
	By     string    `json:"by"`
}

// HowToCall is how to reach an agent: a CLI command, and links where it
// has them.
type HowToCall struct {
	CLI     string `json:"cli"`
	A2ACard string `json:"a2a_card,omitempty"`
	Profile string `json:"profile,omitempty"`
	MCP     string `json:"mcp,omitempty"`
}

type Capability struct {
	ID           string                 `json:"id"`
	AgentID      string                 `json:"agent_id"`
	Name         string                 `json:"name"`
	Description  string                 `json:"description"`
	InputSchema  map[string]interface{} `json:"input_schema"`
	OutputSchema map[string]interface{} `json:"output_schema"`
	Visibility   Visibility             `json:"visibility"`
	CreatedAt    time.Time              `json:"created_at"`
	UpdatedAt    time.Time              `json:"updated_at"`
	// PublicInvoke = true when the owner has opted this capability
	// into being callable without a friendship/grant (migration 0022).
	// When true, PublicMonthlyQuotaPerAgent carries the per-invoker
	// monthly cap.
	PublicInvoke bool `json:"public_invoke"`
	// PublicMonthlyQuotaPerAgent — per-invoker monthly quota (calendar
	// month). Non-nil only when PublicInvoke is true.
	PublicMonthlyQuotaPerAgent *int32 `json:"public_monthly_quota_per_agent"`
	// Semantics is SemanticsAutonomous or SemanticsHumanInLoop.
	Semantics string `json:"semantics,omitempty"`
	// Consent: whether requests wait for the owner's approval.
	Consent *CapabilityConsent `json:"consent,omitempty"`
}

// CapabilityConsent is a capability's owner consent settings.
type CapabilityConsent struct {
	Required bool `json:"required"`
	// Approvers are user ids who may decide; empty means the account's
	// owners and admins. Only shown to those who manage the agent.
	Approvers []string `json:"approvers,omitempty"`
	// MaxValidity is the longest standing consent: "once", "1h", "1d",
	// "7d" or "30d".
	MaxValidity string `json:"max_validity"`
	// Wait is how long a held request waits: "<n>m", "<n>h" or "<n>d".
	Wait string `json:"wait"`
}

// ConsentSettings is `consent` on create and update; nil fields keep
// their value.
type ConsentSettings struct {
	Required *bool `json:"required,omitempty"`
	// Approvers are user ids or emails of members of the agent's account.
	Approvers   []string `json:"approvers,omitempty"`
	MaxValidity string   `json:"max_validity,omitempty"`
	// Wait is "<n>m", "<n>h" or "<n>d", from 5m to 7d (default 24h).
	Wait string `json:"wait,omitempty"`
}

// ConsentTerms is what a caller proposes when a capability asks its
// owner first. The owner may approve it, narrow it or deny it.
type ConsentTerms struct {
	Purpose string `json:"purpose,omitempty"`
	// ValidFor is "once" (default), "<n>h" (1-720) or "<n>d" (1-30).
	ValidFor    string `json:"valid_for,omitempty"`
	MaxRequests int    `json:"max_requests,omitempty"`
}

// InvocationConsent is what the owner approved, and the consent token
// proving it.
type InvocationConsent struct {
	ConsentRequestID string       `json:"consent_request_id"`
	ConsentID        *string      `json:"consent_id"`
	ApprovedAt       *time.Time   `json:"approved_at"`
	Terms            ConsentTerms `json:"terms"`
	// Token is an EdDSA JWT (typ "chakramcp-consent+jwt") signed with the
	// relay's key; nil until delivered.
	Token *string `json:"token"`
}

// HumanConfirmed says a person confirmed the reply (human_in_loop).
type HumanConfirmed struct {
	ConsentRequestID string     `json:"consent_request_id"`
	ConfirmedAt      *time.Time `json:"confirmed_at"`
	Token            *string    `json:"token"`
}

// DraftedReply is a reply to a human_in_loop capability, kept as a
// draft until one of the agent's owners confirms it at ApproveURL.
type DraftedReply struct {
	Status           string    `json:"status"` // "awaiting_confirmation"
	ConsentRequestID string    `json:"consent_request_id"`
	ApproveURL       string    `json:"approve_url"`
	ExpiresAt        time.Time `json:"expires_at"`
	// Safety is what message safety removed or flagged in the draft.
	Safety *Safety `json:"safety,omitempty"`
}

// Notice kinds.
const (
	NoticeConsentRequested = "consent_requested"
	NoticeReplyConfirmed   = "reply_confirmed"
	NoticeReplyRejected    = "reply_rejected"
	NoticeReplyExpired     = "reply_expired"
)

// Notice is something an agent is told out of band of its inbox. A
// NoticeConsentRequested notice means a call waits for your owner's
// approval: pass ConsentRequest.ApproveURL to them. Never includes the
// call's input.
type Notice struct {
	ID             string                `json:"id"`
	Kind           string                `json:"kind"`
	CreatedAt      time.Time             `json:"created_at"`
	ConsentRequest *NoticeConsentRequest `json:"consent_request,omitempty"`
	// InvocationID is the invocation the notice is about. Nil for a
	// notice about none, and from relays before 0.7.
	InvocationID *string `json:"invocation_id,omitempty"`
}

type NoticeConsentRequest struct {
	ID             string       `json:"id"`
	InvocationID   string       `json:"invocation_id"`
	Status         string       `json:"status"`
	CapabilityID   string       `json:"capability_id"`
	CapabilityName *string      `json:"capability_name"`
	Requester      Requester    `json:"requester"`
	Terms          ConsentTerms `json:"terms"`
	ExpiresAt      time.Time    `json:"expires_at"`
	ApproveURL     string       `json:"approve_url"`
}

// Requester is the agent asking, and whether its account is verified.
type Requester struct {
	AgentID            string  `json:"agent_id"`
	AgentDisplayName   *string `json:"agent_display_name"`
	AccountID          string  `json:"account_id"`
	AccountDisplayName *string `json:"account_display_name"`
	Verified           bool    `json:"verified"`
}

type AgentSummary struct {
	ID                 string `json:"id"`
	Slug               string `json:"slug"`
	DisplayName        string `json:"display_name"`
	AccountID          string `json:"account_id"`
	AccountSlug        string `json:"account_slug"`
	AccountDisplayName string `json:"account_display_name"`
}

type Friendship struct {
	ID              string           `json:"id"`
	Status          FriendshipStatus `json:"status"`
	Proposer        AgentSummary     `json:"proposer"`
	Target          AgentSummary     `json:"target"`
	ProposerMessage *string          `json:"proposer_message"`
	ResponseMessage *string          `json:"response_message"`
	CounterOfID     *string          `json:"counter_of_id"`
	CreatedAt       time.Time        `json:"created_at"`
	UpdatedAt       time.Time        `json:"updated_at"`
	DecidedAt       *time.Time       `json:"decided_at"`
	IProposed       bool             `json:"i_proposed"`
	IReceived       bool             `json:"i_received"`
}

type Grant struct {
	ID                   string       `json:"id"`
	Status               GrantStatus  `json:"status"`
	Granter              AgentSummary `json:"granter"`
	Grantee              AgentSummary `json:"grantee"`
	CapabilityID         string       `json:"capability_id"`
	CapabilityName       string       `json:"capability_name"`
	CapabilityVisibility Visibility   `json:"capability_visibility"`
	GrantedAt            time.Time    `json:"granted_at"`
	ExpiresAt            *time.Time   `json:"expires_at"`
	RevokedAt            *time.Time   `json:"revoked_at"`
	RevokeReason         *string      `json:"revoke_reason"`
	Purpose              *string      `json:"purpose"` // why the granter issued it; nil when unset
	IGranted             bool         `json:"i_granted"`
	IReceived            bool         `json:"i_received"`
}

type InvokeResponse struct {
	InvocationID string           `json:"invocation_id"`
	Status       InvocationStatus `json:"status"`
	Error        *string          `json:"error"`
	// ConsentRequestID is set when the capability asks its owner first:
	// the request is held (InvocationAwaitingConsent) or a standing
	// consent covered it (InvocationPending).
	ConsentRequestID *string `json:"consent_request_id,omitempty"`
	// Code says why a rejected call was refused, when message safety
	// refused it: "chk.policy.safety_blocked" or
	// "chk.policy.compliance_denied".
	Code string `json:"code,omitempty"`
	// Safety is what message safety removed, flagged or blocked.
	Safety *Safety `json:"safety,omitempty"`
}

// SafetyFinding is one thing message safety found, and what it did. It
// never holds the removed value: only its kind and where it was.
type SafetyFinding struct {
	// Kind is "email", "phone", "url", "messaging_link", "handle",
	// "off_platform", "opaque", "unjudged", or a question id such as
	// "harm_crime".
	Kind string `json:"kind"`
	// Action is "redacted", "flagged" or "blocked".
	Action string `json:"action"`
	// Path is where in the message, e.g. "input.note".
	Path string `json:"path,omitempty"`
	// Question and P are set when the model judged it.
	Question string   `json:"question,omitempty"`
	P        *float64 `json:"p,omitempty"`
}

// Safety is what message safety did to a call: Request for the input,
// Reply for the reply (shown to the caller once it's released).
type Safety struct {
	Request []SafetyFinding `json:"request,omitempty"`
	Reply   []SafetyFinding `json:"reply,omitempty"`
}

type Invocation struct {
	ID                 string           `json:"id"`
	GrantID            *string          `json:"grant_id"`
	GranterAgentID     *string          `json:"granter_agent_id"`
	GranterDisplayName *string          `json:"granter_display_name"`
	GranteeAgentID     *string          `json:"grantee_agent_id"`
	GranteeDisplayName *string          `json:"grantee_display_name"`
	CapabilityID       *string          `json:"capability_id"`
	CapabilityName     string           `json:"capability_name"`
	Status             InvocationStatus `json:"status"`
	ElapsedMs          int32            `json:"elapsed_ms"`
	ErrorMessage       *string          `json:"error_message"`
	InputPreview       json.RawMessage  `json:"input_preview"`
	OutputPreview      json.RawMessage  `json:"output_preview"`
	CreatedAt          time.Time        `json:"created_at"`
	ClaimedAt          *time.Time       `json:"claimed_at"`
	IServed            bool             `json:"i_served"`
	IInvoked           bool             `json:"i_invoked"`
	// Tier identifies which trust path authorised this invocation.
	// "friend" = rode a friendship + grant (FriendshipContext /
	// GrantContext are populated on inbox responses). "public" = rode
	// a public_invoke=true capability (migration 0022); contexts come
	// back nil. Use this instead of inspecting GrantID or the contexts
	// directly — it removes the "nil means public" footgun for
	// handlers. Empty on relays predating PR #145.
	Tier string `json:"tier"`
	// Trust context for this invocation. Two sources:
	//
	//   - On Inbox.Pull responses, the relay just verified friendship
	//     + grant before delivering this row — handlers can trust
	//     these assertions without re-querying.
	//
	//   - On audit-log endpoints (Invocations.List/Get), for rows
	//     queued on/after PR #146, the contexts come from a frozen
	//     JSONB snapshot captured at queue time. They reflect what was
	//     true when the call was queued, not necessarily what's true
	//     now.
	//
	// nil on public-invoke rows (no friendship/grant existed) and on
	// legacy pre-snapshot rows.
	FriendshipContext *FriendshipContext `json:"friendship_context,omitempty"`
	GrantContext      *GrantContext      `json:"grant_context,omitempty"`
	// Semantics of the capability: SemanticsHumanInLoop routes the
	// invocation to ServeOptions.HumanHandler.
	Semantics string `json:"semantics,omitempty"`
	// Consent is what the owner approved, for a capability that asks
	// first.
	Consent *InvocationConsent `json:"consent,omitempty"`
	// HumanConfirmed says a person confirmed the reply, for a
	// human_in_loop capability.
	HumanConfirmed *HumanConfirmed `json:"human_confirmed,omitempty"`
	// Safety is what message safety removed, flagged or blocked.
	Safety *Safety `json:"safety,omitempty"`
	// CallID is the call this invocation was made in, if any.
	CallID *string `json:"call_id,omitempty"`
}

type FriendshipContext struct {
	ID              string           `json:"id"`
	Status          FriendshipStatus `json:"status"`
	ProposerAgentID string           `json:"proposer_agent_id"`
	TargetAgentID   string           `json:"target_agent_id"`
	ProposerMessage *string          `json:"proposer_message"`
	ResponseMessage *string          `json:"response_message"`
	DecidedAt       *time.Time       `json:"decided_at"`
}

type GrantContext struct {
	ID                   string      `json:"id"`
	Status               GrantStatus `json:"status"`
	GranterAgentID       string      `json:"granter_agent_id"`
	GranteeAgentID       string      `json:"grantee_agent_id"`
	CapabilityID         string      `json:"capability_id"`
	CapabilityName       string      `json:"capability_name"`
	CapabilityVisibility Visibility  `json:"capability_visibility"`
	GrantedAt            time.Time   `json:"granted_at"`
	ExpiresAt            *time.Time  `json:"expires_at"`
	// Purpose is why the granter issued this grant; nil when unset or
	// when the relay/snapshot predates grant purposes.
	Purpose *string `json:"purpose,omitempty"`
}

// ─── Request bodies ──────────────────────────────────────

type CreateAgentRequest struct {
	AccountID   string     `json:"account_id"`
	Slug        string     `json:"slug"`
	DisplayName string     `json:"display_name"`
	Description string     `json:"description,omitempty"`
	Visibility  Visibility `json:"visibility,omitempty"`
	EndpointURL *string    `json:"endpoint_url,omitempty"`
	// Tags are directory tags: at most 10, each 1-32 lowercase letters,
	// digits or hyphens.
	Tags []string `json:"tags,omitempty"`
}

type UpdateAgentRequest struct {
	DisplayName string     `json:"display_name,omitempty"`
	Description string     `json:"description,omitempty"`
	Visibility  Visibility `json:"visibility,omitempty"`
	// AcceptCalls sets who may call the agent; empty keeps it.
	AcceptCalls AcceptCalls `json:"accept_calls,omitempty"`
	// Tags replaces the directory tags; nil keeps them, and a pointer to
	// an empty slice removes them all.
	Tags *[]string `json:"tags,omitempty"`
}

type CreateCapabilityRequest struct {
	Name         string                 `json:"name"`
	Description  string                 `json:"description,omitempty"`
	InputSchema  map[string]interface{} `json:"input_schema,omitempty"`
	OutputSchema map[string]interface{} `json:"output_schema,omitempty"`
	Visibility   Visibility             `json:"visibility,omitempty"`
	// PublicInvoke (migration 0022): when *true, makes the capability
	// callable without a friendship/grant. Requires Visibility == Network
	// and PublicMonthlyQuotaPerAgent != nil. Pointer so the zero value
	// doesn't accidentally set it false on every PATCH.
	PublicInvoke               *bool  `json:"public_invoke,omitempty"`
	PublicMonthlyQuotaPerAgent *int32 `json:"public_monthly_quota_per_agent,omitempty"`
	// Semantics: SemanticsHumanInLoop makes a reply a draft until an
	// owner confirms it. Empty means autonomous.
	Semantics string `json:"semantics,omitempty"`
	// Consent: ask the owner before requests reach the agent (pull-mode
	// agents only).
	Consent *ConsentSettings `json:"consent,omitempty"`
}

type UpdateCapabilityRequest struct {
	Description                *string                `json:"description,omitempty"`
	InputSchema                map[string]interface{} `json:"input_schema,omitempty"`
	OutputSchema               map[string]interface{} `json:"output_schema,omitempty"`
	Visibility                 Visibility             `json:"visibility,omitempty"`
	PublicInvoke               *bool                  `json:"public_invoke,omitempty"`
	PublicMonthlyQuotaPerAgent *int32                 `json:"public_monthly_quota_per_agent,omitempty"`
	Semantics                  string                 `json:"semantics,omitempty"`
	Consent                    *ConsentSettings       `json:"consent,omitempty"`
}

type ProposeFriendshipRequest struct {
	ProposerAgentID string  `json:"proposer_agent_id"`
	TargetAgentID   string  `json:"target_agent_id"`
	ProposerMessage *string `json:"proposer_message,omitempty"`
}

type CreateGrantRequest struct {
	GranterAgentID string     `json:"granter_agent_id"`
	GranteeAgentID string     `json:"grantee_agent_id"`
	CapabilityID   string     `json:"capability_id"`
	ExpiresAt      *time.Time `json:"expires_at,omitempty"`
	// Purpose is an optional free-text reason the grantee needs access.
	// The relay trims it (empty becomes null) and caps it at 500 chars.
	Purpose *string `json:"purpose,omitempty"`
}

// InvokeRequest carries exactly one of GrantID (trusted path) or
// CapabilityID (public-invoke path, migration 0022). The relay
// returns 400 if you send both or neither. Both are `omitempty` so a
// caller fills only the one it's using.
type InvokeRequest struct {
	GrantID        string          `json:"grant_id,omitempty"`
	CapabilityID   string          `json:"capability_id,omitempty"`
	GranteeAgentID string          `json:"grantee_agent_id"`
	Input          json.RawMessage `json:"input"`
	// ConsentTerms, for a capability that asks its owner first.
	ConsentTerms *ConsentTerms `json:"consent_terms,omitempty"`
	// CallID makes the invocation part of an active call: the invoking
	// agent must be a party to it, and the capability the other
	// party's.
	CallID string `json:"call_id,omitempty"`
}

// HandlerResult is what an inbox.serve handler returns.
type HandlerResult struct {
	Status string          `json:"status"` // "succeeded" or "failed"
	Output json.RawMessage `json:"output,omitempty"`
	Error  string          `json:"error,omitempty"`
}

// Succeeded is a convenience constructor for HandlerResult.
func Succeeded(output any) HandlerResult {
	b, err := json.Marshal(output)
	if err != nil {
		return HandlerResult{Status: "failed", Error: err.Error()}
	}
	return HandlerResult{Status: "succeeded", Output: b}
}

// Failed is a convenience constructor for HandlerResult.
func Failed(err string) HandlerResult {
	return HandlerResult{Status: "failed", Error: err}
}

// ─── Reviews (migration 0023) ──────────────────────────────

// ReviewTier is either "friend" (the reviewer has an accepted
// friendship with the target at write-time) or "public" (no
// friendship; reviewer reached the target via a public-invokable
// capability).
type ReviewTier string

const (
	ReviewTierFriend ReviewTier = "friend"
	ReviewTierPublic ReviewTier = "public"
)

type ReviewTag struct {
	CapabilityID   string `json:"capability_id"`
	CapabilityName string `json:"capability_name"`
}

type Review struct {
	ID        string       `json:"id"`
	Reviewer  AgentSummary `json:"reviewer"`
	Target    AgentSummary `json:"target"`
	Rating    int16        `json:"rating"`
	Comment   *string      `json:"comment"`
	Tier      ReviewTier   `json:"tier"`
	Tags      []ReviewTag  `json:"tags"`
	Hidden    bool         `json:"hidden"`
	CreatedAt time.Time    `json:"created_at"`
	UpdatedAt time.Time    `json:"updated_at"`
	// IAuthored is true when the caller owns the reviewer agent.
	IAuthored bool `json:"i_authored"`
}

// ReviewDistribution maps star bucket → count. Mirrors the relay's
// JSON shape where keys are stringified bucket numbers.
type ReviewDistribution struct {
	One   int64 `json:"1"`
	Two   int64 `json:"2"`
	Three int64 `json:"3"`
	Four  int64 `json:"4"`
	Five  int64 `json:"5"`
}

type ReviewSummary struct {
	// Average is the mean rating over the *visible* set. Nil when
	// Count == 0.
	Average      *float64           `json:"average"`
	Count        int64              `json:"count"`
	Distribution ReviewDistribution `json:"distribution"`
}

type ReviewListResponse struct {
	Reviews    []Review      `json:"reviews"`
	NextCursor *string       `json:"next_cursor,omitempty"`
	Summary    ReviewSummary `json:"summary"`
}

type ReviewListQuery struct {
	Cursor        string
	Limit         int
	Tier          ReviewTier
	IncludeHidden bool
}

type WriteReviewRequest struct {
	ReviewerAgentID     string   `json:"reviewer_agent_id"`
	Rating              int16    `json:"rating"`
	Comment             *string  `json:"comment,omitempty"`
	TaggedCapabilityIDs []string `json:"tagged_capability_ids"`
}

type EligibleReviewer struct {
	ReviewerAgentID      string   `json:"reviewer_agent_id"`
	ReviewerDisplayName  string   `json:"reviewer_display_name"`
	TagableCapabilityIDs []string `json:"tagable_capability_ids"`
}

type EligibilityResponse struct {
	Eligible []EligibleReviewer `json:"eligible"`
}

// ─── Calls ────────────────────────────────────────────────

// CallState is where a call is: ringing until the callee answers,
// then active until it ends. CallDeclined, CallMissed and CallEnded
// are final.
type CallState string

const (
	CallRinging  CallState = "ringing"
	CallActive   CallState = "active"
	CallDeclined CallState = "declined"
	CallMissed   CallState = "missed"
	CallEnded    CallState = "ended"
)

// CallMemoryMode is who remembers a call: CallMemoryCall (the relay
// keeps its memory) or CallMemoryOwn (each agent remembers for
// itself; the default).
type CallMemoryMode string

const (
	CallMemoryCall CallMemoryMode = "call"
	CallMemoryOwn  CallMemoryMode = "own"
)

// CallEntryKind is what a transcript entry records.
type CallEntryKind string

const (
	CallEntryInvite     CallEntryKind = "invite"
	CallEntryMessage    CallEntryKind = "message"
	CallEntryAccepted   CallEntryKind = "accepted"
	CallEntryDeclined   CallEntryKind = "declined"
	CallEntryEnded      CallEntryKind = "ended"
	CallEntryMissed     CallEntryKind = "missed"
	CallEntryInvocation CallEntryKind = "invocation"
)

// CallParty is one side of a call, as it stands now.
type CallParty struct {
	// AgentID is nil once the agent is deleted.
	AgentID *string `json:"agent_id"`
	// AccountID is the agent's current account (the account at invite
	// once the agent is gone).
	AccountID   string  `json:"account_id"`
	DisplayName *string `json:"display_name"`
	AccountSlug *string `json:"account_slug"`
}

// CallMemoryFeatures is what call memory can do on this call, fixed at
// invite: Tags (pinned decisions and open questions) and Recall (Ask).
type CallMemoryFeatures struct {
	Tags   bool `json:"tags"`
	Recall bool `json:"recall"`
}

// Call is a conversation between two agents. Its terms (Memory,
// TTLSecs, IdleSecs, Topic) are set by the caller and fixed.
type Call struct {
	ID             string             `json:"id"`
	State          CallState          `json:"state"`
	Caller         CallParty          `json:"caller"`
	Callee         CallParty          `json:"callee"`
	Memory         CallMemoryMode     `json:"memory"`
	MemoryFeatures CallMemoryFeatures `json:"memory_features"`
	// TTLSecs is how long the content is kept after the call ends.
	TTLSecs int64 `json:"ttl_secs"`
	// IdleSecs: the call ends after this long without messages.
	IdleSecs   int64      `json:"idle_secs"`
	Topic      *string    `json:"topic"`
	CreatedAt  time.Time  `json:"created_at"`
	AnsweredAt *time.Time `json:"answered_at"`
	EndedAt    *time.Time `json:"ended_at"`
	// EndedBy is "caller", "callee", "owner", "idle" or "gone".
	EndedBy *string `json:"ended_by"`
	// PurgeAt is when the content will be deleted; PurgedAt when it was.
	PurgeAt       *time.Time `json:"purge_at"`
	PurgedAt      *time.Time `json:"purged_at"`
	MessageCount  int64      `json:"message_count"`
	LastMessageAt *time.Time `json:"last_message_at"`
}

// CallInvocation is the invocation a CallEntryInvocation entry
// records. Its result is read from the invocation itself.
type CallInvocation struct {
	ID         string            `json:"id"`
	Capability *string           `json:"capability"`
	Status     *InvocationStatus `json:"status"`
}

// CallEntry is one entry of a call's transcript.
type CallEntry struct {
	Seq         int64         `json:"seq"`
	Kind        CallEntryKind `json:"kind"`
	FromAgentID *string       `json:"from_agent_id"`
	// Text is nil for events, blocked messages and purged content.
	Text *string         `json:"text"`
	Data json.RawMessage `json:"data,omitempty"`
	// Blocked: message safety refused it; it has no content.
	Blocked bool `json:"blocked"`
	// Safety is what message safety removed, flagged or blocked.
	Safety     []SafetyFinding `json:"safety,omitempty"`
	Invocation *CallInvocation `json:"invocation,omitempty"`
	CreatedAt  time.Time       `json:"created_at"`
}

// MemoryTags is what Jev judged about a message, each a probability
// except Weight (0, chit-chat, to 3, an agreed term or deadline).
type MemoryTags struct {
	Decision     float64 `json:"decision"`
	Commitment   float64 `json:"commitment"`
	OpenQuestion float64 `json:"open_question"`
	Fact         float64 `json:"fact"`
	Weight       float64 `json:"weight"`
	// Resolves is the Seq of the earlier question this message answers,
	// if any, and ResolvesP how likely it answers it.
	Resolves  *int     `json:"resolves,omitempty"`
	ResolvesP *float64 `json:"resolves_p,omitempty"`
}

// MemoryInvocation is an invocation made in the call, as the memory
// shows it. Input and Output are set once it has finished.
type MemoryInvocation struct {
	ID         string           `json:"id"`
	Capability string           `json:"capability"`
	Status     InvocationStatus `json:"status"`
	Input      json.RawMessage  `json:"input,omitempty"`
	Output     json.RawMessage  `json:"output,omitempty"`
}

// MemoryItem is a message (Text) or an invocation (Invocation) in a
// call's memory, quoted verbatim.
type MemoryItem struct {
	Seq int64 `json:"seq"`
	// From is the sending agent's id.
	From *string   `json:"from"`
	At   time.Time `json:"at"`
	Text *string   `json:"text,omitempty"`
	// Context is the previous message, on a short pinned one ("Agreed."
	// with what was agreed).
	Context    *string           `json:"context,omitempty"`
	Tags       *MemoryTags       `json:"tags,omitempty"`
	Invocation *MemoryInvocation `json:"invocation,omitempty"`
}

// CallMemory is a call's memory in call-memory mode: Pinned decisions,
// commitments and facts, Open questions, and the Recent messages.
type CallMemory struct {
	Pinned []MemoryItem `json:"pinned"`
	Open   []MemoryItem `json:"open"`
	Recent []MemoryItem `json:"recent"`
}

// RecallMessage is a message Ask found, with P, how likely it answers
// the question.
type RecallMessage struct {
	Seq  int64     `json:"seq"`
	From *string   `json:"from"`
	Text *string   `json:"text"`
	At   time.Time `json:"at"`
	P    float64   `json:"p"`
}

// CallRecall is what Ask returns: Answered, how likely any of Messages
// answers the question (nil when there was nothing to search), and up
// to five messages, best first.
type CallRecall struct {
	Answered *float64        `json:"answered"`
	Messages []RecallMessage `json:"messages"`
}

// CallFeedItem is one entry of an agent's calls feed, with its call.
// Memory is set in call-memory mode on the newest message of each call
// in a pull.
type CallFeedItem struct {
	Call   Call        `json:"call"`
	Entry  CallEntry   `json:"entry"`
	Memory *CallMemory `json:"memory,omitempty"`
}

// CallList is a page of calls, newest first.
type CallList struct {
	Calls      []Call  `json:"calls"`
	NextCursor *string `json:"next_cursor,omitempty"`
}

// StartCallRequest invites ToAgentID to a call from FromAgentID, one
// of your agents.
type StartCallRequest struct {
	FromAgentID string `json:"from_agent_id"`
	ToAgentID   string `json:"to_agent_id"`
	// Memory: empty means CallMemoryOwn.
	Memory CallMemoryMode `json:"memory,omitempty"`
	// TTLSecs is how long the content is kept after the call ends; nil
	// takes the relay's default (30 days), and 0 is allowed.
	TTLSecs *int64 `json:"ttl_secs,omitempty"`
	// IdleSecs ends the call after this long without messages, 3,600
	// to 604,800; 0 takes the default (24 h).
	IdleSecs int64  `json:"idle_secs,omitempty"`
	Topic    string `json:"topic,omitempty"`
	// Message is an opening message, sent with the invite.
	Message string `json:"message,omitempty"`
}

// CallMessage is a message to send on a call: text, and optional JSON
// data. Together at most 16 KiB.
type CallMessage struct {
	Text string          `json:"text"`
	Data json.RawMessage `json:"data,omitempty"`
}
