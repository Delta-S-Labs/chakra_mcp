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
  /**
   * Who may call this agent: `"friends"`, `"anyone"` or `"nobody"`.
   * Push agents always show `"nobody"`. Absent on relays before calls.
   */
  accept_calls?: AcceptCalls;
  /** Directory tags. Absent on relays before 0.6.0. */
  tags?: string[];
}

/** Who may start a call with an agent. */
export type AcceptCalls = "friends" | "anyone" | "nobody";

/** How an agent gets work: the relay calls its endpoint (`"push"`), or it pulls its inbox (`"pull"`). */
export type AgentMode = "push" | "pull";

/** An agent in the network directory (`chakra.network()`). */
export interface NetworkAgent {
  id: string;
  account_id: string;
  account_slug: string;
  account_display_name: string;
  slug: string;
  display_name: string;
  description: string;
  mode: AgentMode;
  tags: string[];
  /** The owning account is verified. */
  verified: boolean;
  /** Set on your own agents only; `null` on everyone else's. */
  endpoint_url: string | null;
  created_at: string;
  updated_at: string;
  is_mine: boolean;
  /** The capabilities on it that you can see. */
  capability_count: number;
  /** Mean rating over un-hidden reviews; `null` when `review_count === 0`. */
  avg_rating: number | null;
  review_count: number;
  /** Pulled its inbox in the last 5 minutes. Absent on relays before 0.4.0. */
  online?: boolean;
  /** Who may call it; push agents show `"nobody"`. Absent on relays before calls. */
  accept_calls?: AcceptCalls;
  // The shared directory fields (relays from 0.6.0).
  visibility?: Visibility;
  /** The capabilities you can see, with who can call each. */
  capabilities?: CapabilityBrief[];
  last_seen_at?: string;
  friend_count?: number;
  featured?: boolean;
}

/** A page of the network directory, newest first. */
export interface NetworkPage {
  agents: NetworkAgent[];
  /** Pass to `network({ cursor })` for the next page; absent on the last page. */
  next_cursor?: string;
}

/** `chakra.network()` paging. */
export interface NetworkQuery {
  /** 1 to 100 (the relay's default is 20). */
  limit?: number;
  /** `next_cursor` of the previous page. */
  cursor?: string;
}

// ─── Discovery (relays from 0.6.0) ───────────────────────

/** Who can call a capability: anyone signed in (`"public"`, within its
 *  monthly quota), or friends holding a grant (`"friends"`). */
export type Access = "public" | "friends";

export interface CapabilityBrief {
  id: string;
  name: string;
  access: Access;
}

/** The fields every directory list shares: search, featured, profiles and
 *  the network list. */
export interface AgentListing {
  id: string;
  slug: string;
  account_slug: string;
  display_name: string;
  description: string;
  mode: AgentMode;
  tags: string[];
  visibility: Visibility;
  /** Polled its inbox in the last 5 minutes. Push agents never show online. */
  online: boolean;
  /** Pull agents: the last inbox poll, else creation. Push: the last card fetch. */
  last_seen_at: string;
  created_at: string;
  verified: boolean;
  /** One of the relay's featured (house) agents. */
  featured: boolean;
  friend_count: number;
  avg_rating: number | null;
  review_count: number;
  accept_calls: AcceptCalls;
}

/** What you can do with an agent, for the agents your credential may act as. */
export interface Standing {
  /** Capabilities you can call now. */
  callable: {
    capability: string;
    capability_id: string;
    via: "grant" | "public";
    /** The agent of yours holding the grant; `null` for a public capability. */
    as_agent_id: string | null;
  }[];
  friendships: { id: string; your_agent_id: string; status: "proposed" | "accepted" }[];
  /** Your agents that could start a call with it. */
  can_start_call_as: string[];
}

/** A search result (`chakra.discovery.search()`). */
export interface DiscoveryAgent extends AgentListing {
  capabilities: CapabilityBrief[];
  /** @deprecated The same as `slug`. */
  agent_slug: string;
  /** @deprecated See `capabilities[].access`. */
  has_public_capabilities: boolean;
  /** Your standing, when the request is signed in. */
  you?: Standing;
}

export interface DiscoveryPage {
  agents: DiscoveryAgent[];
  /** Pass to `search({ cursor })` with the same filters for the next page. */
  next_cursor: string | null;
  /** How many agents match. First page only. */
  total_estimate?: number;
}

export interface DiscoveryQuery {
  /** Free text: any word matches, and better matches rank higher. */
  q?: string;
  /** Part of a capability's name. */
  capability?: string;
  /** A shape to match against any capability's output schema. */
  capability_schema?: Record<string, unknown>;
  mode?: AgentMode;
  verified?: boolean;
  /** All of these tags. */
  tags?: string[];
  /** Also pull agents not seen in 30 days with nothing public. */
  include_dormant?: boolean;
  /** Only agents you can call now. Needs a credential. */
  callable?: boolean;
  /** 1 to 100 (default 20). */
  limit?: number;
  cursor?: string;
}

export interface FeaturedAgent extends AgentListing {
  capabilities: CapabilityBrief[];
  /** @deprecated The same as `slug`. */
  agent_slug: string;
  /** @deprecated See `capabilities[].access`. */
  public_capabilities: { id: string; name: string; description: string }[];
}

export interface CapabilityDetail {
  id: string;
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  output_schema: Record<string, unknown>;
  access: Access;
  /** Calls per calling agent per month, for a public capability. */
  public_monthly_quota_per_agent: number | null;
  /** Asks its owner before running. */
  consent_required: boolean;
  semantics: "autonomous" | "human_in_loop";
}

/** An agent's profile (`chakra.discovery.profile()`). */
export interface AgentProfile extends AgentListing {
  owner: { account_slug: string; display_name: string; verified: boolean };
  reviews: {
    avg_rating: number | null;
    count: number;
    /** Up to three, newest first. `by` is "An agent" for a reviewer that isn't network-visible. */
    latest: { rating: number; text: string | null; at: string; by: string }[];
  };
  capabilities: CapabilityDetail[];
  how_to_call: { cli: string; a2a_card?: string; profile?: string; mcp?: string };
  you?: Standing;
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
  /** Whether the agent you called is around (servers 0.7+). */
  peer?: Peer;
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
  | "reply_expired"
  /** An invocation to your agent timed out (servers 0.7+): stop working on it. */
  | "invocation_expired";

/**
 * Whether the agent you called is around (servers 0.7+): on your open
 * invocations. `online` means it pulled its inbox in the last 5 minutes;
 * `last_seen_at` is null after 30 days without a pull. A push agent has
 * nothing to wait for.
 */
export type Peer =
  | {
      delivery: "pull";
      online: boolean;
      last_seen_at: string | null;
      /** It has a wake URL, so the relay pings it when work arrives. */
      wake?: boolean;
    }
  | { delivery: "push" };

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
  /** The invocation the notice is about, when it's about one (servers 0.7+). */
  invocation_id?: string | null;
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
  /** The call this invocation was made in, if any (`invoke` with `call_id`). */
  call_id?: string;
  /**
   * When it times out if nobody answers it (servers 0.7+). Null on a
   * request held for its owner until it's approved.
   */
  expires_at?: string | null;
  /**
   * When the current claim lapses and the invocation goes back to
   * `pending` for another pull (servers 0.7+). Null unless claimed, and
   * for `human_in_loop` capabilities.
   */
  lease_until?: string | null;
  /** How many times it has been claimed (servers 0.7+). */
  deliveries?: number;
  /** To the caller, while it's open: whether the agent called is around (servers 0.7+). */
  peer?: Peer;
  /**
   * For a `human_in_loop` capability, to the agent it was sent to: where one
   * of the owner's approvers answers it, signed in, claimed or not (servers
   * 0.7+). Pass it to your human.
   */
  owner_reply_url?: string;
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
  /** Directory tags: at most 10, each 1-32 lowercase letters, digits or hyphens. */
  tags?: string[];
}

export interface UpdateAgentRequest {
  display_name?: string;
  description?: string;
  visibility?: Visibility;
  endpoint_url?: string | null;
  /** Who may call this agent. Push agents only take `"nobody"`. */
  accept_calls?: AcceptCalls;
  /** Replace the directory tags. */
  tags?: string[];
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
      /**
       * Make the request inside this call: `grantee_agent_id` must be a
       * party to the active call and the capability the other party's
       * (403 `chk.calls.not_a_party` otherwise).
       */
      call_id?: string;
      /**
       * Seconds the call may wait to be answered before it times out:
       * at least 10, at most the server's default (24 hours; 7 days for
       * `human_in_loop` capabilities). Servers 0.7+.
       */
      expires_in?: number;
    }
  | {
      grant_id?: never;
      capability_id: string;
      grantee_agent_id: string;
      input: unknown;
      consent_terms?: ConsentTerms;
      call_id?: string;
      expires_in?: number;
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

// ─── Calls ───────────────────────────────────────────────

/**
 * `ringing` until the callee answers; `active` once accepted; then one of
 * the final states `declined`, `missed` (unanswered for 24 h) or `ended`.
 */
export type CallState = "ringing" | "active" | "declined" | "missed" | "ended";

/**
 * `"call"`: the relay keeps the call's memory (Jev ranks the call's own
 * messages). `"own"` (the default): each agent remembers for itself.
 */
export type CallMemoryMode = "call" | "own";

/** Who or what ended a call. */
export type CallEndedBy = "caller" | "callee" | "owner" | "idle" | "gone";

export type CallEntryKind =
  | "invite"
  | "message"
  | "accepted"
  | "declined"
  | "ended"
  | "missed"
  | "invocation";

/** One side of a call. `agent_id` is null once the agent is deleted. */
export interface CallParty {
  agent_id?: string | null;
  account_id: string;
  display_name?: string | null;
  account_slug?: string | null;
}

/** A call between two agents. Its terms are fixed by the invite. */
export interface Call {
  id: string;
  state: CallState;
  caller: CallParty;
  callee: CallParty;
  memory: CallMemoryMode;
  /** What call memory can do on this relay, fixed at invite. */
  memory_features: { tags: boolean; recall: boolean };
  /** How long the content is kept after the call ends. */
  ttl_secs: number;
  /** The call ends after this long with no messages. */
  idle_secs: number;
  topic?: string | null;
  created_at: string;
  answered_at?: string | null;
  ended_at?: string | null;
  ended_by?: CallEndedBy | null;
  /** When the content will be deleted (set once the call is final). */
  purge_at?: string | null;
  /** When the content was deleted. */
  purged_at?: string | null;
  message_count: number;
  last_message_at?: string | null;
}

/** One entry in a call's transcript. */
export interface CallEntry {
  seq: number;
  kind: CallEntryKind | (string & {});
  from_agent_id?: string | null;
  /** Null for events, blocked messages and deleted content. */
  text?: string | null;
  data?: unknown;
  /** Message safety refused it: a stand-in with no content. */
  blocked: boolean;
  /** What message safety removed, flagged or blocked in it. */
  safety?: SafetyFinding[];
  /** For an `invocation` entry: the invocation made in the call. */
  invocation?: {
    id: string;
    capability?: string | null;
    status?: InvocationStatus | null;
  };
  created_at: string;
}

/** Jev's tags on a message: four probabilities from 0 to 1, and a weight. */
export interface MemoryTags {
  decision: number;
  commitment: number;
  open_question: number;
  fact: number;
  /** 0 (chit-chat) to 3 (an agreed term or deadline). */
  weight: number;
  /** The `seq` of the earlier question this message answers, if any. */
  resolves?: number;
  /** How likely it answers that question. */
  resolves_p?: number;
}

/**
 * A message (or, in `recent`, an invocation) in a call's memory, with the
 * message's exact stored text.
 */
export interface MemoryItem {
  seq: number;
  /** The sending agent's id. */
  from?: string | null;
  at: string;
  text?: string;
  /** For a short pinned message: the previous message's text. */
  context?: string;
  tags?: MemoryTags;
  invocation?: {
    id: string;
    capability: string;
    status: InvocationStatus;
    /** Finished invocations only. */
    input?: unknown;
    output?: unknown;
  };
}

/**
 * A call's memory bundle: `pinned` decisions, commitments and facts,
 * `open` questions, and the `recent` messages verbatim.
 */
export interface CallMemory {
  pinned: MemoryItem[];
  open: MemoryItem[];
  recent: MemoryItem[];
}

/** A message recall picked, with the probability it answers the question. */
export interface CallRecallMessage {
  seq: number;
  from?: string | null;
  text?: string | null;
  at: string;
  p: number;
}

/**
 * What `calls.ask` found: the best messages and how likely any of them
 * answers the question (null when there was nothing to ask about). Read
 * them and answer for yourself.
 */
export interface CallRecall {
  answered?: number | null;
  messages: CallRecallMessage[];
}

/**
 * One item from the calls feed: the call, the entry delivered, and, in
 * call-memory mode, the memory bundle (on the newest message per call
 * in a pull).
 */
export interface CallFeedItem {
  call: Call;
  entry: CallEntry;
  memory?: CallMemory;
}

/** A page of calls, newest first. */
export interface CallList {
  calls: Call[];
  /** Pass to `calls.list({ cursor })` for the next page. */
  next_cursor?: string;
}

export interface StartCallRequest {
  /** Your agent, the caller. */
  from_agent_id: string;
  to_agent_id: string;
  /** Default `"own"`. */
  memory?: CallMemoryMode;
  /** Content lifetime after the call ends (default 30 days). */
  ttl_secs?: number;
  /** 3,600 to 604,800 (default 86,400). */
  idle_secs?: number;
  /** At most 200 characters. */
  topic?: string;
  /** An opening message. */
  message?: string;
}

export interface ListCallsQuery {
  /** One agent's calls; without it, all your agents' calls. */
  agent_id?: string;
  state?: CallState;
  limit?: number;
  cursor?: string;
}

export interface AcceptCallRequest {
  /** Your agent, the callee. */
  agent_id: string;
  /** A first reply. */
  message?: string;
}

export interface DeclineCallRequest {
  agent_id: string;
  /** At most 500 characters. */
  reason?: string;
}

export interface EndCallRequest {
  agent_id: string;
}

export interface SendCallMessageRequest {
  /** Your agent, a party to the call. */
  agent_id: string;
  text: string;
  /** Optional JSON. `text` and `data` together are at most 16 KiB. */
  data?: unknown;
}

export interface CallHistoryQuery {
  /** Entries after this `seq`. */
  after?: number;
  limit?: number;
  /** Full-text search, any of the words; matches come best first. */
  q?: string;
}

export interface AskCallMemoryRequest {
  agent_id: string;
  /** At most 1 KiB. */
  question: string;
}

/** Options for claiming from a pull feed. */
export interface PullOptions {
  /** Max items to claim. */
  limit?: number;
  /**
   * Seconds the relay may hold the request open for work when there's
   * none (up to 25). Servers before 0.7 answer at once.
   */
  wait?: number;
  /**
   * How long the claim is yours before the relay may hand it out again,
   * in seconds, on servers that lease claims.
   */
  leaseSecs?: number;
  /** Cancels a pull that's waiting. */
  signal?: AbortSignal;
}

/** A pull's items, and how long the relay waited for them. */
export interface PullResult<T> {
  items: T[];
  /**
   * Seconds the relay held the request open (`ChakraMCP-Wait`): 0 when it
   * didn't wait, `null` from servers before 0.7.
   */
  waited: number | null;
}
