package chakramcp

import (
	"encoding/json"
	"time"
)

type Visibility string

const (
	VisibilityPrivate Visibility = "private"
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
	ID                  string     `json:"id"`
	AccountID           string     `json:"account_id"`
	AccountSlug         string     `json:"account_slug"`
	AccountDisplayName  string     `json:"account_display_name"`
	Slug                string     `json:"slug"`
	DisplayName         string     `json:"display_name"`
	Description         string     `json:"description"`
	Visibility          Visibility `json:"visibility"`
	EndpointURL         *string    `json:"endpoint_url"`
	CreatedAt           time.Time  `json:"created_at"`
	UpdatedAt           time.Time  `json:"updated_at"`
	IsMine              bool       `json:"is_mine"`
	CapabilityCount     int64      `json:"capability_count"`
}

type Capability struct {
	ID            string                 `json:"id"`
	AgentID       string                 `json:"agent_id"`
	Name          string                 `json:"name"`
	Description   string                 `json:"description"`
	InputSchema   map[string]interface{} `json:"input_schema"`
	OutputSchema  map[string]interface{} `json:"output_schema"`
	Visibility    Visibility             `json:"visibility"`
	CreatedAt     time.Time              `json:"created_at"`
	UpdatedAt     time.Time              `json:"updated_at"`
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
	IGranted             bool         `json:"i_granted"`
	IReceived            bool         `json:"i_received"`
}

type InvokeResponse struct {
	InvocationID string           `json:"invocation_id"`
	Status       InvocationStatus `json:"status"`
	Error        *string          `json:"error"`
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
	// Trust context bundled by the relay on Inbox.Pull responses only.
	// The relay just verified friendship + grant before delivering this
	// row — handlers can trust these assertions without re-querying.
	// nil on audit-log endpoints (Invocations.List/Get).
	FriendshipContext *FriendshipContext `json:"friendship_context,omitempty"`
	GrantContext      *GrantContext      `json:"grant_context,omitempty"`
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
}

// ─── Request bodies ──────────────────────────────────────

type CreateAgentRequest struct {
	AccountID   string     `json:"account_id"`
	Slug        string     `json:"slug"`
	DisplayName string     `json:"display_name"`
	Description string     `json:"description,omitempty"`
	Visibility  Visibility `json:"visibility,omitempty"`
	EndpointURL *string    `json:"endpoint_url,omitempty"`
}

type UpdateAgentRequest struct {
	DisplayName string     `json:"display_name,omitempty"`
	Description string     `json:"description,omitempty"`
	Visibility  Visibility `json:"visibility,omitempty"`
}

type CreateCapabilityRequest struct {
	Name         string                 `json:"name"`
	Description  string                 `json:"description,omitempty"`
	InputSchema  map[string]interface{} `json:"input_schema,omitempty"`
	OutputSchema map[string]interface{} `json:"output_schema,omitempty"`
	Visibility   Visibility             `json:"visibility,omitempty"`
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
}

type InvokeRequest struct {
	GrantID         string          `json:"grant_id"`
	GranteeAgentID  string          `json:"grantee_agent_id"`
	Input           json.RawMessage `json:"input"`
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
