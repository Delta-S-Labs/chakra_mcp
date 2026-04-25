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
}

export interface MeResponse {
  user: User;
  memberships: Membership[];
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
  /** Plaintext — shown exactly once on creation. */
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

export const apiBaseUrl = BASE;
