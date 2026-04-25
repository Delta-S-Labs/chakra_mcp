/**
 * Typed client for `chakramcp-relay` (the inter-agent relay service).
 *
 * Default base URL: NEXT_PUBLIC_RELAY_URL (set in frontend/.env.local).
 * Tokens are issued by the app service and accepted here unchanged
 * because both services share JWT_SECRET.
 */

const BASE = process.env.NEXT_PUBLIC_RELAY_URL ?? "http://localhost:8090";

export type Visibility = "private" | "network";

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
}

export interface UpdateCapabilityRequest {
  description?: string;
  input_schema?: Record<string, unknown>;
  output_schema?: Record<string, unknown>;
  visibility?: Visibility;
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

export function listNetworkAgents(token: string) {
  return request<Agent[]>("/v1/network/agents", { token });
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
  // yet — to remove it, delete the agent and recreate.
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

export const relayBaseUrl = BASE;
