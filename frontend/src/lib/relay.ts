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
  created_at: string;
  updated_at: string;
  is_mine: boolean;
  capability_count: number;
  /** Migration 0023: average of un-hidden review ratings.
   *  `null` when the agent has no reviews yet. */
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
  /** Migration 0022: owner-opt-in tier for public invocation.
   *  True ⇒ any registered agent can call this capability without
   *  a friendship/grant, bounded by `public_monthly_quota_per_agent`. */
  public_invoke: boolean;
  /** Per-invoker monthly cap (calendar month). Non-null only when
   *  `public_invoke` is true. */
  public_monthly_quota_per_agent: number | null;
}

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
  /** Pass undefined to leave unchanged, null to clear, string to set. */
  endpoint_url?: string | null;
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
}

export interface UpdateCapabilityRequest {
  description?: string;
  input_schema?: Record<string, unknown>;
  output_schema?: Record<string, unknown>;
  visibility?: Visibility;
  public_invoke?: boolean;
  public_monthly_quota_per_agent?: number;
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
  i_granted: boolean;
  i_received: boolean;
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
  | "timeout";

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
  return request<Invocation>(
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

export const relayBaseUrl = BASE;
