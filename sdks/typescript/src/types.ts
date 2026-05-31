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
  i_granted: boolean;
  i_received: boolean;
}

export type InvocationStatus =
  | "pending"
  | "in_progress"
  | "succeeded"
  | "failed"
  | "rejected"
  | "timeout";

export interface InvokeResponse {
  invocation_id: string;
  status: InvocationStatus;
  error: string | null;
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
   * HITL semantics of the underlying capability (issue #69). Populated
   * on `inbox.pull` responses so `InboxClient.serve()` can route
   * `human_in_loop` invocations to a `humanHandler` instead of an
   * autonomous handler — autonomous replies on a HITL capability are
   * rejected by the relay with 409 `chk.policy.requires_human_confirmation`.
   *
   * NOTE: as of PR 4 (TS SDK), the relay's `InvocationDto` does not yet
   * serialise this field — that's a small backend follow-up. The SDK is
   * forward-compatible: once the relay returns `semantics`, routing
   * lights up automatically; until then the SDK treats every invocation
   * as autonomous (the relay's gate still catches HITL mistakes — the
   * worker just sees a 409 at result-post time instead of a clean route).
   */
  semantics?: "autonomous" | "human_in_loop";
  /**
   * Trust context bundled by the relay on `inbox.pull` responses only.
   * The relay just verified friendship + grant before delivering this
   * row - your handler can trust these assertions without re-querying.
   * Always undefined on audit-log endpoints (`invocations.list/get`).
   */
  friendship_context?: FriendshipContext;
  grant_context?: GrantContext;
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
   * HITL gate (issue #69 PR 2). `"autonomous"` (default) accepts any
   * worker-posted result. `"human_in_loop"` forces the relay to reject
   * the result with 409 `chk.policy.requires_human_confirmation`
   * unless the response carries `confirmed_by_human: true`. PR 4 will
   * plumb a `humanHandler` callback through `InboxClient.serve()`;
   * until then, set this manually only on capabilities you handle
   * out-of-band via the CLI.
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
    }
  | {
      grant_id?: never;
      capability_id: string;
      grantee_agent_id: string;
      input: unknown;
    };

export type HandlerResult =
  | { status: "succeeded"; output: unknown }
  | { status: "failed"; error: string };

export class ChakraMCPError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
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
  ) {
    super(429, "monthly_quota_exhausted", message);
    this.name = "QuotaExhaustedError";
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
