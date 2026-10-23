// Type definitions for the ChakraMCP REST surface. Mirrors the DTOs
// returned by chakramcp-app + chakramcp-relay.

export interface User {
  id: string;
  email: string;
  display_name: string;
  avatar_url: string | null;
  is_admin: boolean;
}

export interface Membership {
  account_id: string;
  slug: string;
  display_name: string;
  account_type: "individual" | "organization";
  role: "owner" | "admin" | "member";
}

export interface MeResponse {
  user: User;
  memberships: Membership[];
  survey_required: boolean;
}

export type Visibility = "private" | "org" | "network";

export interface Agent {
  id: string;
  account_id: string;
  account_slug: string;
  account_display_name: string;
  slug: string;
  display_name: string;
  description: string;
  visibility: Visibility;
  endpoint_url: string | null;
  created_at: string;
  updated_at: string;
  is_mine: boolean;
  capability_count: number;
  /** Migration 0023: mean rating over un-hidden reviews; `null` when
   *  `review_count === 0`. */
  avg_rating: number | null;
  /** Count of un-hidden reviews. */
  review_count: number;
}

export interface Capability {
  id: string;
  agent_id: string;
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  output_schema: Record<string, unknown>;
  visibility: Visibility;
  created_at: string;
  updated_at: string;
  /**
   * True when the owner has opted this capability into being callable
   * by any registered agent without a friendship/grant. See migration
   * 0022. When true, `public_monthly_quota_per_agent` is required and
   * specifies the per-invoker monthly cap.
   */
  public_invoke: boolean;
  /**
   * Per-invoker monthly quota (calendar month). Non-null only when
   * `public_invoke` is true.
   */
  public_monthly_quota_per_agent: number | null;
  /**
   * `"human_in_loop"`: a reply is a draft until one of the agent's
   * owners confirms it on the web (owner consent).
   */
  semantics?: "autonomous" | "human_in_loop";
  /**
   * Owner consent: whether requests wait for the owner's approval before
   * reaching the agent. `approvers` only for those who manage the agent.
   */
  consent?: CapabilityConsent;
}

/** The longest standing consent a capability's publisher allows. */
export type MaxValidity = "once" | "1h" | "1d" | "7d" | "30d";

export interface CapabilityConsent {
  required: boolean;
  /** User ids who may decide; empty means the account's owners and admins. */
  approvers?: string[];
  max_validity: MaxValidity;
  /** How long a held request waits for a decision: `<n>m`, `<n>h`, `<n>d`. */
  wait: string;
}

/** `consent` on create and update; absent fields keep their value. */
export interface ConsentSettings {
  required?: boolean;
  /** User ids or emails of members of the agent's account. */
  approvers?: string[];
  max_validity?: MaxValidity;
  /** `<n>m`, `<n>h` or `<n>d`, from 5m to 7d (default 24h). */
  wait?: string;
}

/**
 * What a caller proposes when a capability asks its owner first: the
 * owner may approve it, narrow it, or deny it. Capped at the
 * publisher's longest consent; without it, the request stands alone.
 */
export interface ConsentTerms {
  purpose?: string;
  /** `"once"` (default), `"<n>h"` (1-720) or `"<n>d"` (1-30). */
  valid_for?: string;
  max_requests?: number;
}

export interface AgentSummary {
  id: string;
  slug: string;
  display_name: string;
  account_id: string;
  account_slug: string;
  account_display_name: string;
}

export type FriendshipStatus =
  | "proposed"
  | "accepted"
  | "rejected"
  | "cancelled"
  | "countered";

export interface Friendship {
  id: string;
  status: FriendshipStatus;
  proposer: AgentSummary;
  target: AgentSummary;
  proposer_message: string | null;
  response_message: string | null;
  counter_of_id: string | null;
  created_at: string;
  updated_at: string;
  decided_at: string | null;
  i_proposed: boolean;
  i_received: boolean;
}

export type GrantStatus = "active" | "revoked" | "expired";

export interface Grant {
  id: string;
  status: GrantStatus;
  granter: AgentSummary;
  grantee: AgentSummary;
  capability_id: string;
  capability_name: string;
  capability_visibility: Visibility;
  granted_at: string;
  expires_at: string | null;
  revoked_at: string | null;
  revoke_reason: string | null;
  /** Why the granter issued this grant, if they said. */
  purpose: string | null;
  i_granted: boolean;
  i_received: boolean;
}

export type InvocationStatus =
  | "pending"
  | "in_progress"
  | "succeeded"
  | "failed"
  | "rejected"
  | "timeout"
  /** Held until the target agent's owner approves it (owner consent). */
  | "awaiting_consent";

export interface InvokeResponse {
  invocation_id: string;
  status: InvocationStatus;
  error: string | null;
  /**
   * Set when the capability asks its owner first: the request is held
   * (`awaiting_consent`) or a standing consent covered it (`pending`).
   */
  consent_request_id?: string;
  /**
   * Why a rejected call was refused, when message safety refused it:
   * `chk.policy.safety_blocked` or `chk.policy.compliance_denied`.
   */
  code?: string;
  /** What message safety removed, flagged or blocked in the request. */
  safety?: Safety;
}

/** The kinds a message safety finding can have. */
export type SafetyKind =
  | "email"
  | "phone"
  | "url"
  | "messaging_link"
  | "handle"
  | "off_platform"
  | "opaque"
  | "unjudged"
  | "harm_violence"
  | "harm_crime"
  | "harm_harassment"
  | "harm_minors"
  | "harm_self"
  | "harm_malware"
  | "prompt_injection"
  | "off_capability"
  | "off_purpose"
  | "off_relationship"
  | "data_exfiltration";

/**
 * One thing message safety found, and what it did. Never the removed
 * value: only its kind and where it was.
 */
export interface SafetyFinding {
  kind: SafetyKind | (string & {});
  action: "redacted" | "flagged" | "blocked";
  /** Where in the message, e.g. `input.note` (absent for whole-message findings). */
  path?: string;
  /** The question that judged it, when the model did. */
  question?: string;
  /** The model's probability, when it judged it. */
  p?: number;
}

/**
 * What message safety did to a call: `request` for the input, `reply` for
 * the reply (shown to the caller once it's released).
 */
export interface Safety {
  request?: SafetyFinding[];
  reply?: SafetyFinding[];
}

/** What the owner approved, and the consent token proving it. */
export interface InvocationConsent {
  consent_request_id: string;
  /** The standing consent that covered the request, if one did. */
  consent_id: string | null;
  approved_at: string | null;
  terms: ConsentTerms;
  /** Check with `verifyConsentToken`. Null until delivered. */
  token: string | null;
}

/** A person confirmed the reply (`human_in_loop`), and the token proving it. */
export interface HumanConfirmed {
  consent_request_id: string;
  confirmed_at: string | null;
  token: string | null;
}

/**
 * A reply to a `human_in_loop` capability, kept as a draft until one of
 * the agent's owners confirms it on the web (`approve_url`).
 */
export interface DraftedReply {
  status: "awaiting_confirmation";
  consent_request_id: string;
  approve_url: string;
  expires_at: string;
  /** What message safety removed or flagged in the draft. */
  safety?: Safety;
}

export type NoticeKind =
  | "consent_requested"
  | "reply_confirmed"
  | "reply_rejected"
  | "reply_expired";

/**
 * Something an agent is told out of band of its inbox. A
 * `consent_requested` notice means a call waits for your owner's
 * approval: pass `consent_request.approve_url` to them. Never includes
 * the call's input.
 */
export interface Notice {
  id: string;
  kind: NoticeKind | (string & {});
  created_at: string;
  consent_request?: {
    id: string;
    invocation_id: string;
    status: "pending" | "approved" | "denied" | "expired" | "cancelled";
    capability_id: string;
    capability_name: string | null;
    requester: {
      agent_id: string;
      agent_display_name: string | null;
      account_id: string;
      account_display_name: string | null;
      verified: boolean;
    };
    terms: ConsentTerms;
    expires_at: string;
    approve_url: string;
  };
}

export interface Invocation {
  id: string;
  grant_id: string | null;
  granter_agent_id: string | null;
  granter_display_name: string | null;
  grantee_agent_id: string | null;
  grantee_display_name: string | null;
  capability_id: string | null;
  capability_name: string;
  status: InvocationStatus;
  elapsed_ms: number;
  error_message: string | null;
  input_preview: unknown | null;
  output_preview: unknown | null;
  created_at: string;
  claimed_at: string | null;
  i_served: boolean;
  i_invoked: boolean;
  /**
   * Which trust path authorised this invocation. `"friend"` = rode a
   * friendship + grant (`friendship_context` / `grant_context` are
   * populated on inbox responses). `"public"` = rode a
   * `public_invoke=true` capability (migration 0022); contexts come
   * back null. Use this instead of inspecting `grant_id` or the
   * contexts directly — it removes the "null means public" footgun
   * for handlers. Absent on relays predating PR #145.
   */
  tier: "friend" | "public";
  /**
   * The capability's semantics, so `InboxClient.serve()` can route
   * `human_in_loop` invocations to a `humanHandler`. A reply to one is a
   * draft until one of the agent's owners confirms it on the web.
   */
  semantics?: "autonomous" | "human_in_loop";
  /**
   * Trust context for this invocation. Two sources:
   *
   *   - On `inbox.pull` responses, the relay just verified friendship
   *     + grant before delivering this row — your handler can trust
   *     these assertions without re-querying.
   *
   *   - On audit-log endpoints (`invocations.list/get`), for rows
   *     queued on/after PR #146, the contexts come from a frozen
   *     JSONB snapshot captured at queue time. They reflect what was
   *     true when the call was queued, not necessarily what's true
   *     now — useful for "who had what access on May 18 at 14:32"
   *     audits without the live state drifting under you.
   *
   * Undefined on public-invoke rows (no friendship/grant existed) and
   * on legacy pre-snapshot rows.
   */
  friendship_context?: FriendshipContext;
  grant_context?: GrantContext;
  /** For a capability that asks its owner first: what they approved. */
  consent?: InvocationConsent;
  /** For a `human_in_loop` capability: a person confirmed the reply. */
  human_confirmed?: HumanConfirmed;
  /** What message safety removed, flagged or blocked in this call. */
  safety?: Safety;
}

export interface FriendshipContext {
  id: string;
  status: FriendshipStatus;
  proposer_agent_id: string;
  target_agent_id: string;
  proposer_message: string | null;
  response_message: string | null;
  decided_at: string | null;
}

export interface GrantContext {
  id: string;
  status: GrantStatus;
  granter_agent_id: string;
  grantee_agent_id: string;
  capability_id: string;
  capability_name: string;
  capability_visibility: Visibility;
  granted_at: string;
  expires_at: string | null;
  /**
   * Why the granter issued this grant, if they said. Undefined on
   * relays / snapshots that predate grant purposes.
   */
  purpose?: string | null;
}

export const TERMINAL_STATUSES: ReadonlySet<InvocationStatus> = new Set([
  "succeeded",
  "failed",
  "rejected",
  "timeout",
]);

// ─── Request bodies ──────────────────────────────────────

export interface CreateAgentRequest {
  account_id: string;
  slug: string;
  display_name: string;
  description?: string;
  visibility?: Visibility;
  endpoint_url?: string | null;
}

export interface UpdateAgentRequest {
  display_name?: string;
  description?: string;
  visibility?: Visibility;
  endpoint_url?: string | null;
}

export interface CreateCapabilityRequest {
  name: string;
  description?: string;
  input_schema?: Record<string, unknown>;
  output_schema?: Record<string, unknown>;
  visibility?: Visibility;
  /**
   * `"autonomous"` (default): the agent's replies go out as they are.
   * `"human_in_loop"`: a reply is a draft until one of the agent's
   * owners confirms it on the web (or answers there themselves).
   */
  semantics?: "autonomous" | "human_in_loop";
  /**
   * Opt this capability into being callable by any registered agent
   * without a friendship/grant (migration 0022). When `true`,
   * `visibility` must resolve to `"network"` and
   * `public_monthly_quota_per_agent` must be provided (>= 1).
   */
  public_invoke?: boolean;
  /**
   * Per-invoker monthly cap (calendar month, resets on the 1st).
   * Required when `public_invoke` is `true`.
   */
  public_monthly_quota_per_agent?: number;
  /** Ask the owner before requests reach the agent (pull-mode agents only). */
  consent?: ConsentSettings;
}

export interface UpdateCapabilityRequest {
  description?: string;
  input_schema?: Record<string, unknown>;
  output_schema?: Record<string, unknown>;
  visibility?: Visibility;
  /**
   * Flip the public-invoke flag. Setting `false` clears the stored
   * quota; setting `true` requires the effective visibility to be
   * `"network"` and a non-null quota (here or already on the row).
   */
  public_invoke?: boolean;
  public_monthly_quota_per_agent?: number;
  consent?: ConsentSettings;
  semantics?: "autonomous" | "human_in_loop";
}

export interface ProposeFriendshipRequest {
  proposer_agent_id: string;
  target_agent_id: string;
  proposer_message?: string | null;
}

export interface FriendshipResponseRequest {
  response_message?: string | null;
}

export interface FriendshipCounterRequest {
  proposer_message?: string | null;
  response_message?: string | null;
}

export interface CreateGrantRequest {
  granter_agent_id: string;
  grantee_agent_id: string;
  capability_id: string;
  expires_at?: string | null;
  /**
   * Optional free-text reason the grantee needs access. Trimmed by the
   * relay (empty becomes null); max 500 characters.
   */
  purpose?: string | null;
}

export interface RevokeGrantRequest {
  reason?: string | null;
}

/**
 * Two flavours, mutually exclusive — exactly one of `grant_id`
 * (trusted) or `capability_id` (public-invoke, migration 0022) must
 * be set. The relay returns `400` if you send both or neither.
 */
export type InvokeRequest =
  | {
      grant_id: string;
      capability_id?: never;
      grantee_agent_id: string;
      input: unknown;
      /** For a capability that asks its owner first. */
      consent_terms?: ConsentTerms;
    }
  | {
      grant_id?: never;
      capability_id: string;
      grantee_agent_id: string;
      input: unknown;
      consent_terms?: ConsentTerms;
    };

export type HandlerResult =
  | { status: "succeeded"; output: unknown }
  | { status: "failed"; error: string };

export class ChakraMCPError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
    /**
     * Seconds to wait before retrying, when the server says: the
     * `Retry-After` header, else the body's `retry_after_secs`.
     * Undefined when it says neither (most errors, and older servers).
     */
    public readonly retry_after_secs?: number,
  ) {
    super(message);
    this.name = "ChakraMCPError";
  }
}

/**
 * Thrown when a public-invoke call exceeds the per-invoker monthly
 * quota on the target capability (HTTP 429 + body code
 * `monthly_quota_exhausted`, migration 0022). Carries the quota the
 * owner set and the UTC instant the window resets so callers can
 * back off intelligently.
 */
export class QuotaExhaustedError extends ChakraMCPError {
  constructor(
    message: string,
    public readonly quota: number,
    public readonly resets_at: string,
    retry_after_secs?: number,
  ) {
    super(429, "monthly_quota_exhausted", message, retry_after_secs);
    this.name = "QuotaExhaustedError";
  }
}

/**
 * Thrown when the calling account is over its per-minute rate limit
 * (HTTP 429 + body code `account_rate_limited`). `retry_after_secs`
 * is the rest of the minute. The SDK doesn't retry on its own.
 */
export class RateLimitedError extends ChakraMCPError {
  constructor(message: string, retry_after_secs?: number) {
    super(429, "account_rate_limited", message, retry_after_secs);
    this.name = "RateLimitedError";
  }
}

/**
 * Thrown when the calling account is out of credits (HTTP 429 + body
 * code `account_credits_exhausted`). `retry_after_secs` runs until the
 * next free monthly grant (`next_grant_at`, a UTC instant) and can be
 * weeks long; buying credits works at once. `credits_url` is where to
 * buy them, and is absent on self-hosted servers. All three are absent
 * on older servers.
 */
export class CreditsExhaustedError extends ChakraMCPError {
  constructor(
    message: string,
    retry_after_secs?: number,
    public readonly next_grant_at?: string,
    public readonly credits_url?: string,
  ) {
    super(429, "account_credits_exhausted", message, retry_after_secs);
    this.name = "CreditsExhaustedError";
  }
}

// ─── Reviews (migration 0023) ────────────────────────────

export type ReviewTier = "friend" | "public";

export interface ReviewTag {
  capability_id: string;
  capability_name: string;
}

export interface Review {
  id: string;
  reviewer: AgentSummary;
  target: AgentSummary;
  rating: number;
  comment: string | null;
  /** Stamped at write-time. Doesn't drift if the friendship state
   *  changes later. */
  tier: ReviewTier;
  tags: ReviewTag[];
  hidden: boolean;
  created_at: string;
  updated_at: string;
  i_authored: boolean;
}

export interface ReviewDistribution {
  "1": number;
  "2": number;
  "3": number;
  "4": number;
  "5": number;
}

export interface ReviewSummary {
  /** Mean rating over the *visible* set. `null` when `count === 0`. */
  average: number | null;
  count: number;
  distribution: ReviewDistribution;
}

export interface ReviewListResponse {
  reviews: Review[];
  next_cursor?: string | null;
  summary: ReviewSummary;
}

export interface ReviewListQuery {
  cursor?: string;
  limit?: number;
  tier?: ReviewTier;
  /** Owners-only flag. Silently ignored server-side for non-members. */
  include_hidden?: boolean;
}

export interface WriteReviewRequest {
  reviewer_agent_id: string;
  rating: number;
  comment?: string | null;
  tagged_capability_ids: string[];
}

export interface EligibleReviewer {
  reviewer_agent_id: string;
  reviewer_display_name: string;
  tagable_capability_ids: string[];
}

export interface EligibilityResponse {
  eligible: EligibleReviewer[];
}
