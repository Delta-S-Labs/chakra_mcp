/**
 * Typed client for `chakramcp-app` (the user-facing API service).
 *
 * Default base URL: NEXT_PUBLIC_APP_API_URL (set in frontend/.env.local).
 * The relay service has its own client (lib/relay.ts) once that ships.
 *
 * On the server, pass an explicit token from the NextAuth session.
 * On the client, use the wrapper hook (TBD) so token refresh is centralized.
 */

const BASE = process.env.NEXT_PUBLIC_APP_API_URL ?? "http://localhost:8080";

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

export interface UpsertResponse {
  user: User;
  memberships: Membership[];
  token: string;
  survey_required: boolean;
}

export interface MeResponse {
  user: User;
  memberships: Membership[];
  survey_required: boolean;
}

export interface Survey {
  use_case: string | null;
  agent_types: string[];
  frameworks: string[];
  scale: string | null;
  notes: string | null;
  completed_at: string;
}

export interface SubmitSurveyRequest {
  use_case: string | null;
  agent_types: string[];
  frameworks: string[];
  scale: string | null;
  notes: string | null;
}

export interface Org {
  id: string;
  slug: string;
  display_name: string;
  account_type: "individual" | "organization";
  role: "owner" | "admin" | "member";
  created_at: string;
}

export interface OrgMember {
  user_id: string;
  email: string;
  display_name: string;
  avatar_url: string | null;
  role: "owner" | "admin" | "member";
  joined_at: string;
}

export interface ApiKey {
  id: string;
  name: string;
  prefix: string;
  account_id: string | null;
  last_used_at: string | null;
  expires_at: string | null;
  revoked_at: string | null;
  created_at: string;
}

export interface CreateApiKeyResponse {
  api_key: ApiKey;
  /** Plaintext - shown exactly once on creation. */
  plaintext: string;
}

export interface AdminUser extends User {
  created_at: string;
}

export interface AdminOrg {
  id: string;
  slug: string;
  display_name: string;
  account_type: "individual" | "organization";
  member_count: number;
  owner_email: string | null;
  created_at: string;
}

export interface AdminApiKey {
  id: string;
  user_email: string;
  name: string;
  prefix: string;
  account_id: string | null;
  last_used_at: string | null;
  expires_at: string | null;
  revoked_at: string | null;
  created_at: string;
}

export class ApiClientError extends Error {
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
    throw new ApiClientError(res.status, "invalid_response", text || res.statusText);
  }

  if (!res.ok) {
    const err = (json as { error?: { code?: string; message?: string } })?.error;
    throw new ApiClientError(
      res.status,
      err?.code ?? "unknown",
      err?.message ?? res.statusText,
    );
  }

  return json as T;
}

// ─── Email + password auth ──────────────────────────────

export interface AuthResponse {
  user: User;
  memberships: Membership[];
  token: string;
}

export function signupWithPassword(args: {
  email: string;
  password: string;
  name: string;
}) {
  return request<AuthResponse>("/v1/auth/signup", {
    method: "POST",
    body: JSON.stringify(args),
  });
}

export function loginWithPassword(args: { email: string; password: string }) {
  return request<AuthResponse>("/v1/auth/login", {
    method: "POST",
    body: JSON.stringify(args),
  });
}

// ─── OAuth-flow upsert + session ────────────────────────

export function upsertUser(args: {
  email: string;
  name: string;
  avatar_url?: string | null;
  provider: string;
  provider_user_id: string;
  raw_profile?: unknown;
}) {
  return request<UpsertResponse>("/v1/users/upsert", {
    method: "POST",
    body: JSON.stringify(args),
  });
}

export function getMe(token: string) {
  return request<MeResponse>("/v1/me", { token });
}

// ─── Survey ──────────────────────────────────────────────

export function getMySurvey(token: string) {
  return request<Survey | null>("/v1/me/survey", { token });
}

export function submitSurvey(token: string, body: SubmitSurveyRequest) {
  return request<Survey>("/v1/me/survey", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

// ─── Orgs ────────────────────────────────────────────────

export function listOrgs(token: string) {
  return request<Org[]>("/v1/orgs", { token });
}

export function createOrg(token: string, body: { slug: string; display_name: string }) {
  return request<Org>("/v1/orgs", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function getOrg(token: string, slug: string) {
  return request<Org>(`/v1/orgs/${encodeURIComponent(slug)}`, { token });
}

export function listMembers(token: string, slug: string) {
  return request<OrgMember[]>(`/v1/orgs/${encodeURIComponent(slug)}/members`, { token });
}

export function createInvite(
  token: string,
  slug: string,
  body: { email: string; role?: "owner" | "admin" | "member" },
) {
  return request<{ id: string; email: string; role: string; expires_at: string; token: string }>(
    `/v1/orgs/${encodeURIComponent(slug)}/invites`,
    { method: "POST", token, body: JSON.stringify(body) },
  );
}

export interface InvitePreview {
  email: string;
  role: "owner" | "admin" | "member";
  org_slug: string;
  org_display_name: string;
  expires_at: string;
}

export function previewInvite(token: string) {
  return request<InvitePreview>(`/v1/invites/${encodeURIComponent(token)}`);
}

export function acceptInvite(authToken: string, inviteToken: string) {
  return request<Org>(`/v1/invites/${encodeURIComponent(inviteToken)}/accept`, {
    method: "POST",
    token: authToken,
  });
}

// ─── API keys ────────────────────────────────────────────

export function listApiKeys(token: string) {
  return request<ApiKey[]>("/v1/api-keys", { token });
}

export function createApiKey(
  token: string,
  body: { name: string; account_id?: string | null; expires_in_days?: number | null },
) {
  return request<CreateApiKeyResponse>("/v1/api-keys", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function revokeApiKey(token: string, id: string) {
  return request<void>(`/v1/api-keys/${encodeURIComponent(id)}`, {
    method: "DELETE",
    token,
  });
}

export function rotateApiKey(token: string, id: string) {
  return request<CreateApiKeyResponse>(
    `/v1/api-keys/${encodeURIComponent(id)}/rotate`,
    { method: "POST", token },
  );
}

// ─── API-key usage ───────────────────────────────────────
//
// Backend at `backend/app/src/handlers/api_keys.rs::usage`. Until the
// relay invoke handler is wired to stamp `api_key_id` on every
// invocation, this endpoint reads zeros — the shape is real but the
// numbers will be empty arrays. Build the UI assuming live data lands.

export interface ApiKeyUsageCapability {
  capability_name: string;
  count: number;
}

export interface ApiKeyUsageAgent {
  agent_id: string;
  agent_slug: string;
  count: number;
}

export interface ApiKeyUsageDaily {
  /** YYYY-MM-DD (naive date from server). */
  date: string;
  count: number;
}

export interface ApiKeyUsage {
  key_id: string;
  from: string;
  to: string;
  total_requests: number;
  by_capability_type: ApiKeyUsageCapability[];
  by_agent: ApiKeyUsageAgent[];
  daily: ApiKeyUsageDaily[];
}

export function getApiKeyUsage(
  token: string,
  id: string,
  range: { from?: string; to?: string } = {},
) {
  const qs = new URLSearchParams();
  if (range.from) qs.set("from", range.from);
  if (range.to) qs.set("to", range.to);
  const tail = qs.toString();
  const path = `/v1/api-keys/${encodeURIComponent(id)}/usage${tail ? `?${tail}` : ""}`;
  return request<ApiKeyUsage>(path, { token });
}

export function getApiKey(token: string, id: string) {
  // The list endpoint already returns the full row; there's no
  // dedicated GET-one, so we filter the list. Cheap at our scale and
  // keeps the API surface honest.
  return listApiKeys(token).then((all) => all.find((k) => k.id === id) ?? null);
}

// ─── Agent re-parenting ──────────────────────────────────

export interface MoveToPersonalResponse {
  agent_id: string;
  old_account_slug: string;
  new_account_slug: string;
  new_agent_slug: string;
}

export function moveAgentToPersonal(token: string, agentId: string) {
  return request<MoveToPersonalResponse>(
    `/v1/agents/${encodeURIComponent(agentId)}/move-to-personal`,
    { method: "POST", token },
  );
}

// ─── Org delete ──────────────────────────────────────────

export function deleteOrg(token: string, slug: string) {
  return request<void>(`/v1/orgs/${encodeURIComponent(slug)}`, {
    method: "DELETE",
    token,
  });
}

// ─── Unified pairings (device-flow / oauth / api_key) ────
//
// Mirror of `backend/app/src/handlers/pairings.rs::PairingDto`. The
// backend returns a `label` (api-key name, or agent display-name hint
// for device flow, or oauth client_name) — no `account_slug` here, but
// `agent_slug` covers the common "what's this paired to" rendering.

export type PairingKind = "device_flow" | "oauth" | "api_key";

export interface Pairing {
  kind: PairingKind;
  id: string;
  agent_id: string | null;
  agent_slug: string | null;
  label: string | null;
  paired_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
}

export function listPairings(token: string) {
  return request<Pairing[]>("/v1/pairings", { token });
}

export function revokePairing(token: string, kind: PairingKind, id: string) {
  return request<void>(
    `/v1/pairings/${encodeURIComponent(kind)}/${encodeURIComponent(id)}/revoke`,
    { method: "POST", token },
  );
}

// ─── Admin ───────────────────────────────────────────────

export function adminListUsers(token: string) {
  return request<AdminUser[]>("/v1/admin/users", { token });
}

export function adminListOrgs(token: string) {
  return request<AdminOrg[]>("/v1/admin/orgs", { token });
}

export function adminListApiKeys(token: string) {
  return request<AdminApiKey[]>("/v1/admin/api-keys", { token });
}

// ─── OAuth (MCP server consent flow) ─────────────────────

export interface OAuthClientPreview {
  client_id: string;
  client_name: string;
  redirect_uris: string[];
  client_uri: string | null;
  scope: string;
}

export function getOAuthClient(clientId: string) {
  return request<OAuthClientPreview>(
    `/oauth/clients/${encodeURIComponent(clientId)}`,
  );
}

export interface IssueCodeRequest {
  client_id: string;
  redirect_uri: string;
  code_challenge: string;
  code_challenge_method?: "S256";
  scope?: string;
}

export interface IssueCodeResponse {
  code: string;
  expires_in: number;
}

export function issueOAuthCode(token: string, body: IssueCodeRequest) {
  return request<IssueCodeResponse>("/oauth/issue-code", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

// ─── OAuth 2.1 Device Authorization Grant (RFC 8628) ───────────────
//
// Powers the agent-pair UX. Backend mirror at
// `backend/app/src/handlers/oauth.rs` § device flow.

export interface DeviceSession {
  persona: string | null;
  agent_slug_hint: string | null;
  agent_display_name_hint: string | null;
  agent_description_hint: string | null;
  expires_at: string;
  status: "pending" | "approved" | "denied" | "expired" | "consumed";
}

export function getDeviceSession(token: string, userCode: string) {
  return request<DeviceSession>(
    `/oauth/device-session/${encodeURIComponent(userCode)}`,
    { token },
  );
}

export interface DeviceApproveRequest {
  user_code: string;
  agent_slug: string;
  agent_display_name: string;
  agent_description?: string;
  agent_visibility?: "private" | "network";
  account_slug?: string;
}

export interface DeviceApproveResponse {
  status: "approved";
  agent_id: string;
  agent_slug: string;
  account_slug: string;
}

export function approveDeviceSession(token: string, body: DeviceApproveRequest) {
  return request<DeviceApproveResponse>("/oauth/device-approve", {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function denyDeviceSession(token: string, userCode: string) {
  return request<void>("/oauth/device-deny", {
    method: "POST",
    token,
    body: JSON.stringify({ user_code: userCode }),
  });
}

export const apiBaseUrl = BASE;
