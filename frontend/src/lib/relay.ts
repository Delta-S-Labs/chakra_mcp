/**
 * Typed client for `chakramcp-relay` (the inter-agent relay service).
 *
 * Default base URL: NEXT_PUBLIC_RELAY_URL (set in frontend/.env.local).
 * Tokens are issued by the app service and accepted here unchanged
 * because both services share JWT_SECRET.
 */

const BASE = process.env.NEXT_PUBLIC_RELAY_URL ?? "http://localhost:8090";

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
  /** `pull` (polls its inbox) or `push` (takes calls at its own A2A
   *  endpoint). Push agents can't ask their owner first. */
  mode?: "pull" | "push";
  created_at: string;
  updated_at: string;
  is_mine: boolean;
  capability_count: number;
  /** Migration 0023: average of un-hidden review ratings.
   *  `null` when the agent has no reviews yet. */
  avg_rating: number | null;
  /** Count of un-hidden reviews. */
  review_count: number;
  /** When it last pulled its inbox. On `GET /v1/agents` only. */
  last_polled_at?: string | null;
  /** Capabilities others can see (`network`, public ones included). On
   *  `GET /v1/agents` only. */
  network_capability_count?: number;
  /** Who may call it: friends (default), anyone, or nobody. Always
   *  `nobody` for push agents. */
  accept_calls?: AcceptCalls;
  /** Directory tags. */
  tags?: string[];
  /** Set on an agent the relay made for MCP calls: `personal`, or `client`
   *  for an MCP client or API key with no agent of its own. */
  default_kind?: "personal" | "client";
}

export type AcceptCalls = "friends" | "anyone" | "nobody";

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
  /** Migration 0022: owner-opt-in tier for public invocation.
   *  True ⇒ any registered agent can call this capability without
   *  a friendship/grant, bounded by `public_monthly_quota_per_agent`. */
  public_invoke: boolean;
  /** Per-invoker monthly cap (calendar month). Non-null only when
   *  `public_invoke` is true. */
  public_monthly_quota_per_agent: number | null;
  /** `human_in_loop`: the agent's owner writes the replies. */
  semantics?: "autonomous" | "human_in_loop";
  /** Owner consent: whether requests wait for the owner's approval. */
  consent?: CapabilityConsent;
}

/** The longest standing consent a publisher allows. */
export type MaxValidity = "once" | "1h" | "1d" | "7d" | "30d";

export interface CapabilityConsent {
  required: boolean;
  /** User ids who may decide; only for those who manage the agent.
   *  Empty means the account's owners and admins. */
  approvers?: string[];
  max_validity: MaxValidity;
  /** How long a held request waits for a decision: `<n>m`, `<n>h`, `<n>d`. */
  wait: string;
}

/** `consent` on publish and update; absent fields keep their value. */
export interface ConsentSettingsInput {
  required?: boolean;
  /** User ids or member emails. */
  approvers?: string[];
  max_validity?: MaxValidity;
  wait?: string;
}

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
  /** Pass undefined to leave unchanged, null to clear, string to set. */
  endpoint_url?: string | null;
  accept_calls?: AcceptCalls;
  /** Replaces the tags. */
  tags?: string[];
}

export interface CreateCapabilityRequest {
  name: string;
  description?: string;
  input_schema?: Record<string, unknown>;
  output_schema?: Record<string, unknown>;
  visibility?: Visibility;
  /** Migration 0022. When true, the capability becomes callable
   *  without a friendship/grant. Requires `visibility` to resolve to
   *  `"network"` and a non-null monthly quota. */
  public_invoke?: boolean;
  public_monthly_quota_per_agent?: number;
  consent?: ConsentSettingsInput;
  /** `human_in_loop`: a person confirms each reply before the caller sees it. */
  semantics?: "autonomous" | "human_in_loop";
}

export interface UpdateCapabilityRequest {
  description?: string;
  input_schema?: Record<string, unknown>;
  output_schema?: Record<string, unknown>;
  visibility?: Visibility;
  public_invoke?: boolean;
  public_monthly_quota_per_agent?: number;
  consent?: ConsentSettingsInput;
  /** `human_in_loop`: a person confirms each reply before the caller sees it. */
  semantics?: "autonomous" | "human_in_loop";
}

export class RelayClientError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(
  path: string,
  init: RequestInit & { token?: string | null } = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  if (!headers.has("content-type") && init.body) {
    headers.set("content-type", "application/json");
  }
  if (init.token) {
    headers.set("authorization", `Bearer ${init.token}`);
  }

  const res = await fetch(`${BASE}${path}`, { ...init, headers, cache: "no-store" });
  if (res.status === 204) return undefined as T;

  const text = await res.text();
  let json: unknown;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    throw new RelayClientError(res.status, "invalid_response", text || res.statusText);
  }

  if (!res.ok) {
    const err = (json as { error?: { code?: string; message?: string } })?.error;
    throw new RelayClientError(
      res.status,
      err?.code ?? "unknown",
      err?.message ?? res.statusText,
    );
  }
  return json as T;
}

// ─── Agents ──────────────────────────────────────────────

export function listMyAgents(token: string) {
  return request<Agent[]>("/v1/agents", { token });
}

export interface NetworkAgent extends Agent {
  /** "push" (has agent_card_url + endpoint) | "pull" (inbox-only). */
  mode: "push" | "pull";
  tags: string[];
  /** Whether the owning account has been verified by the relay operator. */
  verified: boolean;
  /** Pulled its inbox in the last 5 minutes. */
  online?: boolean;
}

export interface NetworkListResponse {
  agents: NetworkAgent[];
  next_cursor?: string | null;
}

/** Filters mirror the public /v1/discovery/agents shape. All optional. */
export interface NetworkListQuery {
  q?: string;
  mode?: "push" | "pull";
  verified?: boolean;
  /** Comma-separated, AND-match. */
  tags?: string;
  cursor?: string;
  limit?: number;
}

export function listNetworkAgents(token: string, query: NetworkListQuery = {}) {
  const params = new URLSearchParams();
  if (query.q) params.set("q", query.q);
  if (query.mode) params.set("mode", query.mode);
  if (query.verified) params.set("verified", "true");
  if (query.tags) params.set("tags", query.tags);
  if (query.cursor) params.set("cursor", query.cursor);
  if (query.limit !== undefined) params.set("limit", String(query.limit));
  const qs = params.toString();
  const path = qs ? `/v1/network/agents?${qs}` : "/v1/network/agents";
  return request<NetworkListResponse>(path, { token });
}

export function getAgent(token: string, id: string) {
  return request<Agent>(`/v1/agents/${encodeURIComponent(id)}`, { token });
}

export function createAgent(token: string, body: CreateAgentRequest) {
  return request<Agent>("/v1/agents", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function updateAgent(token: string, id: string, body: UpdateAgentRequest) {
  // Only include keys the caller is changing; absent keys are left alone
  // server-side. Note: clearing endpoint_url via null isn't supported
  // yet - to remove it, delete the agent and recreate.
  const payload: Record<string, unknown> = {};
  if (body.display_name !== undefined) payload.display_name = body.display_name;
  if (body.description !== undefined) payload.description = body.description;
  if (body.visibility !== undefined) payload.visibility = body.visibility;
  if (body.endpoint_url !== undefined && body.endpoint_url !== null) {
    payload.endpoint_url = body.endpoint_url;
  }
  if (body.accept_calls !== undefined) payload.accept_calls = body.accept_calls;
  return request<Agent>(`/v1/agents/${encodeURIComponent(id)}`, {
    method: "PATCH",
    token,
    body: JSON.stringify(payload),
  });
}

export function deleteAgent(token: string, id: string) {
  return request<void>(`/v1/agents/${encodeURIComponent(id)}`, {
    method: "DELETE",
    token,
  });
}

// ─── Capabilities ────────────────────────────────────────

export function listCapabilities(token: string, agentId: string) {
  return request<Capability[]>(
    `/v1/agents/${encodeURIComponent(agentId)}/capabilities`,
    { token },
  );
}

export function createCapability(
  token: string,
  agentId: string,
  body: CreateCapabilityRequest,
) {
  return request<Capability>(
    `/v1/agents/${encodeURIComponent(agentId)}/capabilities`,
    { method: "POST", token, body: JSON.stringify(body) },
  );
}

export function updateCapability(
  token: string,
  agentId: string,
  capId: string,
  body: UpdateCapabilityRequest,
) {
  return request<Capability>(
    `/v1/agents/${encodeURIComponent(agentId)}/capabilities/${encodeURIComponent(capId)}`,
    { method: "PATCH", token, body: JSON.stringify(body) },
  );
}

export function deleteCapability(token: string, agentId: string, capId: string) {
  return request<void>(
    `/v1/agents/${encodeURIComponent(agentId)}/capabilities/${encodeURIComponent(capId)}`,
    { method: "DELETE", token },
  );
}

// ─── Friendships ─────────────────────────────────────────

export type FriendshipStatus =
  | "proposed"
  | "accepted"
  | "rejected"
  | "cancelled"
  | "countered";

export interface AgentSummary {
  id: string;
  slug: string;
  display_name: string;
  account_id: string;
  account_slug: string;
  account_display_name: string;
}

export interface AutoFriendshipOrgRef {
  account_id: string;
  slug: string;
  display_name: string;
}

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
  /** Set when this row was created by the per-org auto-friendship
   *  policy (see backend chakramcp_shared::auto_friendship). Used by
   *  the friendships UI to render an "AUTO · via OrgName" badge so
   *  users can tell auto-created rows from user-proposed ones. */
  auto_friendship_org?: AutoFriendshipOrgRef | null;
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

export function listFriendships(
  token: string,
  opts: { direction?: "all" | "outbound" | "inbound"; status?: FriendshipStatus } = {},
) {
  const qs = new URLSearchParams();
  if (opts.direction) qs.set("direction", opts.direction);
  if (opts.status) qs.set("status", opts.status);
  const q = qs.toString();
  return request<Friendship[]>(`/v1/friendships${q ? `?${q}` : ""}`, { token });
}

export function getFriendship(token: string, id: string) {
  return request<Friendship>(`/v1/friendships/${encodeURIComponent(id)}`, { token });
}

export function proposeFriendship(token: string, body: ProposeFriendshipRequest) {
  return request<Friendship>("/v1/friendships", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function acceptFriendship(token: string, id: string, body: FriendshipResponseRequest = {}) {
  return request<Friendship>(`/v1/friendships/${encodeURIComponent(id)}/accept`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function rejectFriendship(token: string, id: string, body: FriendshipResponseRequest = {}) {
  return request<Friendship>(`/v1/friendships/${encodeURIComponent(id)}/reject`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function counterFriendship(token: string, id: string, body: FriendshipCounterRequest) {
  return request<Friendship>(`/v1/friendships/${encodeURIComponent(id)}/counter`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function cancelFriendship(token: string, id: string) {
  return request<Friendship>(`/v1/friendships/${encodeURIComponent(id)}/cancel`, {
    method: "POST",
    token,
    body: JSON.stringify({}),
  });
}

// ─── Grants ──────────────────────────────────────────────

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
  /** Free-text "why" the granter gave for this grant. Null when none
   *  was supplied. */
  purpose: string | null;
  i_granted: boolean;
  i_received: boolean;
}

export interface CreateGrantRequest {
  granter_agent_id: string;
  grantee_agent_id: string;
  capability_id: string;
  expires_at?: string | null;
  /** Optional "why" for the grant (max 500 chars; trimmed, empty → null). */
  purpose?: string | null;
}

export interface RevokeGrantRequest {
  reason?: string | null;
}

export function listGrants(
  token: string,
  opts: { direction?: "all" | "outbound" | "inbound"; status?: GrantStatus } = {},
) {
  const qs = new URLSearchParams();
  if (opts.direction) qs.set("direction", opts.direction);
  if (opts.status) qs.set("status", opts.status);
  const q = qs.toString();
  return request<Grant[]>(`/v1/grants${q ? `?${q}` : ""}`, { token });
}

export function getGrant(token: string, id: string) {
  return request<Grant>(`/v1/grants/${encodeURIComponent(id)}`, { token });
}

export function createGrant(token: string, body: CreateGrantRequest) {
  return request<Grant>("/v1/grants", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function revokeGrant(token: string, id: string, body: RevokeGrantRequest = {}) {
  return request<Grant>(`/v1/grants/${encodeURIComponent(id)}/revoke`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

// ─── Invoke + inbox + audit log ──────────────────────────

export type InvocationStatus =
  | "pending"
  | "in_progress"
  | "succeeded"
  | "failed"
  | "rejected"
  | "timeout"
  /** Held until the target agent's owner approves it (owner consent). */
  | "awaiting_consent";

export interface InvokeRequest {
  grant_id: string;
  grantee_agent_id: string;
  input: unknown;
}

/** Returned synchronously by POST /v1/invoke - the work is enqueued. */
export interface InvokeResponse {
  invocation_id: string;
  status: InvocationStatus;
  error: string | null;
}

/** Friendship state the relay froze at queue time (migration 0024).
 *  Present on audit-log rows queued on/after that migration whose call
 *  rode a friendship+grant; null on public-invoke + legacy rows. */
export interface FriendshipContext {
  id: string;
  status: FriendshipStatus;
  proposer_agent_id: string;
  target_agent_id: string;
  proposer_message: string | null;
  response_message: string | null;
  decided_at: string | null;
}

/** Grant state the relay froze at queue time (migration 0024). */
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
  /** The grant's purpose at queue time. Absent on snapshots taken
   *  before the relay recorded purposes. */
  purpose?: string | null;
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
  /** Which trust path authorised the call: "friend" (rode a
   *  friendship+grant) or "public" (public_invoke capability).
   *  Absent on relays predating the tier field (#145). */
  tier?: "friend" | "public";
  /** Frozen trust context (migration 0024). On audit-log endpoints
   *  these reflect state AT QUEUE TIME, not now — so a since-revoked
   *  grant still shows as it was when the call was authorised. Absent
   *  on public-invoke + pre-snapshot rows. */
  friendship_context?: FriendshipContext;
  grant_context?: GrantContext;
  /** What the owner approved, and the consent token proving it; once a
   *  held request is approved. */
  consent?: InvocationConsent;
  /** For a `human_in_loop` capability: a person confirmed the reply, and
   *  the consent token proving it. */
  human_confirmed?: {
    consent_request_id: string;
    confirmed_at: string | null;
    token: string | null;
  };
  /** What message safety removed, flagged or blocked: the request's, and
   *  the reply's once it's released. Absent when there's nothing. */
  safety?: Safety;
}

/** One thing message safety found, and what it did. Never the removed
 *  value: only its kind and where it was. */
export interface SafetyFinding {
  kind: string;
  action: "redacted" | "flagged" | "blocked";
  path?: string;
  question?: string;
  p?: number;
}

export interface Safety {
  request?: SafetyFinding[];
  reply?: SafetyFinding[];
}

/** A reply to a `human_in_loop` capability, kept as a draft until one of
 *  the agent's approvers confirms it. */
export interface DraftedReply {
  status: "awaiting_confirmation";
  consent_request_id: string;
  approve_url: string;
  expires_at: string;
  /** What message safety removed or flagged in the draft. */
  safety?: Safety;
}

export interface InvocationConsent {
  consent_request_id: string;
  consent_id: string | null;
  approved_at: string | null;
  terms: ConsentTerms;
  /** Null until the inbox delivers the invocation. */
  token: string | null;
}

export interface ReportResultRequest {
  status: "succeeded" | "failed";
  output?: unknown;
  error?: string | null;
}

export const TERMINAL_STATUSES: ReadonlySet<InvocationStatus> = new Set([
  "succeeded",
  "failed",
  "rejected",
  "timeout",
]);

export function invoke(token: string, body: InvokeRequest) {
  return request<InvokeResponse>("/v1/invoke", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function pullInbox(token: string, agentId: string, limit?: number) {
  const qs = new URLSearchParams({ agent_id: agentId });
  if (limit) qs.set("limit", String(limit));
  return request<Invocation[]>(`/v1/inbox?${qs.toString()}`, { token });
}

export function reportResult(
  token: string,
  invocationId: string,
  body: ReportResultRequest,
) {
  return request<Invocation | DraftedReply>(
    `/v1/invocations/${encodeURIComponent(invocationId)}/result`,
    { method: "POST", token, body: JSON.stringify(body) },
  );
}

export function getInvocation(token: string, id: string) {
  return request<Invocation>(`/v1/invocations/${encodeURIComponent(id)}`, { token });
}

export function listInvocations(
  token: string,
  opts: {
    direction?: "all" | "outbound" | "inbound";
    agent_id?: string;
    status?: InvocationStatus;
  } = {},
) {
  const qs = new URLSearchParams();
  if (opts.direction) qs.set("direction", opts.direction);
  if (opts.agent_id) qs.set("agent_id", opts.agent_id);
  if (opts.status) qs.set("status", opts.status);
  const q = qs.toString();
  return request<Invocation[]>(`/v1/invocations${q ? `?${q}` : ""}`, { token });
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
  /** Stamped at write-time. Won't drift if the friendship state
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
  /** Owners-only flag. Silently ignored for non-target-members. */
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

export function listReviews(
  token: string,
  targetAgentId: string,
  query: ReviewListQuery = {},
) {
  const qs = new URLSearchParams();
  if (query.cursor) qs.set("cursor", query.cursor);
  if (query.limit !== undefined) qs.set("limit", String(query.limit));
  if (query.tier) qs.set("tier", query.tier);
  if (query.include_hidden) qs.set("include_hidden", "true");
  const q = qs.toString();
  return request<ReviewListResponse>(
    `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews${q ? `?${q}` : ""}`,
    { token },
  );
}

export function writeReview(
  token: string,
  targetAgentId: string,
  body: WriteReviewRequest,
) {
  return request<Review>(
    `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews`,
    { method: "POST", token, body: JSON.stringify(body) },
  );
}

export function getReviewEligibility(token: string, targetAgentId: string) {
  return request<EligibilityResponse>(
    `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews/eligibility`,
    { token },
  );
}

export function hideReview(
  token: string,
  targetAgentId: string,
  reviewId: string,
) {
  return request<Review>(
    `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews/${encodeURIComponent(reviewId)}/hide`,
    { method: "POST", token, body: "{}" },
  );
}

export function unhideReview(
  token: string,
  targetAgentId: string,
  reviewId: string,
) {
  return request<Review>(
    `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews/${encodeURIComponent(reviewId)}/unhide`,
    { method: "POST", token, body: "{}" },
  );
}

// ─── Audit + usage events (migration 0025) ───────────────

export interface AuditEvent {
  id: string;
  created_at: string;
  actor_user_id: string | null;
  account_id: string | null;
  action: string;
  resource_type: string;
  resource_id: string | null;
  target_id: string | null;
  summary: string;
  metadata: Record<string, unknown>;
}

export interface AuditListResponse {
  events: AuditEvent[];
  next_cursor?: string;
}

export function listAuditEvents(
  token: string,
  opts: { limit?: number; resource_type?: string; action?: string; cursor?: string } = {},
) {
  const qs = new URLSearchParams();
  if (opts.limit) qs.set("limit", String(opts.limit));
  if (opts.resource_type) qs.set("resource_type", opts.resource_type);
  if (opts.action) qs.set("action", opts.action);
  if (opts.cursor) qs.set("cursor", opts.cursor);
  const q = qs.toString();
  return request<AuditListResponse>(`/v1/audit${q ? `?${q}` : ""}`, { token });
}

export interface UsageEvent {
  id: string;
  created_at: string;
  actor_user_id: string | null;
  account_id: string | null;
  surface: string;
  action: string;
  method: string;
  route: string;
  status_code: number;
  ok: boolean;
}

export interface UsageActionCount {
  action: string;
  count: number;
}

export interface UsageEventsResponse {
  events: UsageEvent[];
  /** Totals over every event the server still keeps (`retention_days`). */
  totals_by_action: UsageActionCount[];
  total: number;
  /** Days of events the server keeps; `null` when it keeps them forever.
   *  Absent on older servers. */
  retention_days?: number | null;
  next_cursor?: string;
}

export function listUsageEvents(
  token: string,
  opts: { limit?: number; surface?: string; cursor?: string } = {},
) {
  const qs = new URLSearchParams();
  if (opts.limit) qs.set("limit", String(opts.limit));
  if (opts.surface) qs.set("surface", opts.surface);
  if (opts.cursor) qs.set("cursor", opts.cursor);
  const q = qs.toString();
  return request<UsageEventsResponse>(`/v1/usage/events${q ? `?${q}` : ""}`, { token });
}

// ─── Featured agents (the house agents) ────────────────────

/** An agent from `GET /v1/discovery/featured`: the featured account's agents
 *  (chakramcp.com's house agents), then online agents anyone can call. */
export interface FeaturedAgent {
  id: string;
  account_slug: string;
  slug: string;
  display_name: string;
  description: string;
  online: boolean;
  featured: boolean;
  public_capabilities: { id: string; name: string; description: string }[];
}

/** Featured agents; empty on any error, so callers can just leave them out.
 *  Public, and cached for 30 seconds. */
export async function getFeaturedAgents(): Promise<FeaturedAgent[]> {
  try {
    const res = await fetch(`${BASE}/v1/discovery/featured`, { next: { revalidate: 30 } });
    if (!res.ok) return [];
    const body = (await res.json()) as { agents?: FeaturedAgent[] };
    return body.agents ?? [];
  } catch {
    return [];
  }
}

export const relayBaseUrl = BASE;

// ─── Owner consent ───────────────────────────────────────

/** What a requester proposes and an approver accepts or narrows. */
export interface ConsentTerms {
  purpose?: string;
  /** `once`, `<n>h` or `<n>d`. */
  valid_for?: string;
  max_requests?: number;
}

export interface ConsentAgentRef {
  id: string;
  display_name: string | null;
}

export type ConsentRequestStatus = "pending" | "approved" | "denied" | "expired" | "cancelled";

/** `GET /v1/approvals/{id}`: the approval page. */
export interface ApprovalView {
  id: string;
  kind: "request" | "reply";
  status: ConsentRequestStatus;
  invocation_id: string;
  requester: {
    agent_id: string;
    agent_display_name: string | null;
    account_id: string;
    account_slug: string | null;
    account_display_name: string | null;
    verified: boolean;
  };
  target: ConsentAgentRef;
  capability: { id: string; name: string | null; description: string | null };
  grant_purpose: string | null;
  tier: "friend" | "public";
  terms: ConsentTerms;
  approved_terms: ConsentTerms | null;
  max_validity: string;
  expires_at: string;
  created_at: string;
  decided_at: string | null;
  decision_note: string | null;
  may_decide: boolean;
  approvers: { user_id: string; display_name: string }[];
  /** The held input (for a reply, the original request's input); only
   *  for those who may decide. */
  input?: unknown;
  /** A reply's draft, and the hash to send when confirming it. */
  draft?: { status: "succeeded" | "failed"; output?: unknown; error?: string };
  draft_sha256?: string;
}

/** A row in the waiting and history lists. */
export interface ApprovalSummary {
  id: string;
  kind: "request" | "reply";
  status: ConsentRequestStatus;
  invocation_id: string;
  capability_name: string | null;
  target: ConsentAgentRef;
  requester_agent_id: string;
  requester_display_name: string | null;
  requester_account_display_name: string | null;
  terms: ConsentTerms;
  approved_terms: ConsentTerms | null;
  expires_at: string;
  created_at: string;
  decided_at: string | null;
  decision_note: string | null;
  decided_by_display_name: string | null;
  /** Approved by a standing consent rather than a person. */
  covered: boolean;
}

/** A standing consent, in the active list. */
export interface ConsentSummary {
  id: string;
  capability_id: string;
  capability_name: string | null;
  target: ConsentAgentRef;
  requester_agent_id: string;
  requester_display_name: string | null;
  requester_account_display_name: string | null;
  purpose: string | null;
  valid_from: string;
  valid_until: string;
  max_requests: number | null;
  used: number;
}

export function listApprovals(token: string, state: "waiting" | "history") {
  return request<ApprovalSummary[]>(`/v1/approvals?state=${state}`, { token });
}

export function listActiveConsents(token: string) {
  return request<ConsentSummary[]>("/v1/approvals?state=active", { token });
}

export function getApproval(token: string, id: string) {
  return request<ApprovalView>(`/v1/approvals/${encodeURIComponent(id)}`, { token });
}

export function approveRequest(
  token: string,
  id: string,
  body: { valid_for?: string; max_requests?: number; draft_sha256?: string } = {},
) {
  return request<{ id: string; status: "approved"; consent_id: string | null }>(
    `/v1/approvals/${encodeURIComponent(id)}/approve`,
    { method: "POST", token, body: JSON.stringify(body) },
  );
}

export function denyRequest(token: string, id: string, note?: string) {
  return request<{ id: string; status: "denied" }>(
    `/v1/approvals/${encodeURIComponent(id)}/deny`,
    { method: "POST", token, body: JSON.stringify(note ? { note } : {}) },
  );
}

export function revokeConsent(token: string, id: string) {
  return request<{ id: string; revoked: true }>(
    `/v1/consents/${encodeURIComponent(id)}/revoke`,
    { method: "POST", token },
  );
}

// ─── Calls ───────────────────────────────────────────────
// Two-way sessions between two agents (calls spec 2026-10-25). Owners read
// them and can end them or delete their content; agents do the talking.

export type CallState = "ringing" | "active" | "declined" | "missed" | "ended";
export type CallEndedBy = "caller" | "callee" | "owner" | "idle" | "gone";

export interface CallParty {
  /** `null` once the agent is deleted. */
  agent_id: string | null;
  account_id: string;
  display_name: string | null;
  account_slug: string | null;
}

export interface Call {
  id: string;
  state: CallState;
  caller: CallParty;
  callee: CallParty;
  /** `call`: the relay keeps the call's memory; `own`: each agent does. */
  memory: "call" | "own";
  memory_features: { tags: boolean; recall: boolean };
  ttl_secs: number;
  idle_secs: number;
  topic: string | null;
  created_at: string;
  answered_at: string | null;
  ended_at: string | null;
  ended_by: CallEndedBy | null;
  /** When the content will be deleted (set once the call is over). */
  purge_at: string | null;
  purged_at: string | null;
  message_count: number;
  last_message_at: string | null;
}

export type CallEntryKind =
  | "invite"
  | "message"
  | "accepted"
  | "declined"
  | "ended"
  | "missed"
  | "invocation";

export interface CallEntry {
  seq: number;
  kind: CallEntryKind;
  from_agent_id: string | null;
  /** `null` once the content is deleted, or for a blocked message. */
  text: string | null;
  data: unknown;
  blocked: boolean;
  safety?: SafetyFinding[];
  invocation?: { id: string; capability: string | null; status: string | null };
  created_at: string;
}

export interface CallMemoryItem {
  seq: number;
  from: string | null;
  at: string;
  text?: string;
  /** For a short message: the one before it, for what it refers to. */
  context?: string;
  tags?: Record<string, number>;
  invocation?: { id: string; capability: string; status: string };
}

export interface CallMemory {
  pinned: CallMemoryItem[];
  open: CallMemoryItem[];
  recent: CallMemoryItem[];
}

export interface CallList {
  calls: Call[];
  next_cursor?: string;
}

export function listCalls(
  token: string,
  query: { agent_id?: string; state?: CallState; limit?: number; cursor?: string } = {},
) {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v !== undefined && v !== "") qs.set(k, String(v));
  }
  const suffix = qs.toString() ? `?${qs}` : "";
  return request<CallList>(`/v1/calls${suffix}`, { token });
}

export function getCall(token: string, id: string) {
  return request<Call>(`/v1/calls/${encodeURIComponent(id)}`, { token });
}

export function listCallMessages(
  token: string,
  id: string,
  query: { after?: number; limit?: number; q?: string } = {},
) {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v !== undefined && v !== "") qs.set(k, String(v));
  }
  const suffix = qs.toString() ? `?${qs}` : "";
  return request<CallEntry[]>(`/v1/calls/${encodeURIComponent(id)}/messages${suffix}`, {
    token,
  });
}

export function getCallMemory(token: string, id: string) {
  return request<CallMemory>(`/v1/calls/${encodeURIComponent(id)}/memory`, { token });
}

/** An owner ends a call (web session only). */
export function ownerEndCall(token: string, id: string) {
  return request<Call>(`/v1/calls/${encodeURIComponent(id)}/owner-end`, {
    method: "POST",
    token,
  });
}

/** An owner deletes a call's content now, for both sides (web session only). */
export function deleteCallContent(token: string, id: string) {
  return request<void>(`/v1/calls/${encodeURIComponent(id)}/content`, {
    method: "DELETE",
    token,
  });
}

/** Whether this relay has calls switched on: its calls routes 404 when off. */
export async function callsEnabled(token: string): Promise<boolean> {
  try {
    await listCalls(token, { limit: 1 });
    return true;
  } catch {
    return false;
  }
}
