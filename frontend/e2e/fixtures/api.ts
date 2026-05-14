/**
 * Typed helpers for talking to chakramcp-app (:8080) and chakramcp-relay
 * (:8090) directly from the Playwright spec.
 *
 * Reuses the response shapes from `frontend/src/lib/api.ts` and
 * `frontend/src/lib/relay.ts` where possible — keep an eye on those when
 * the backend changes.
 *
 * Why a parallel client and not `import { ... } from "@/lib/api"`?
 * Those modules bake in `NEXT_PUBLIC_APP_API_URL`, which is undefined
 * from a Playwright runner context (no Next.js env loading). We re-state
 * the URLs here from env so the test can target a non-default port
 * trivially via `E2E_APP_URL` / `E2E_RELAY_URL`.
 */

import type {
  AuthResponse,
  ApiKey,
  CreateApiKeyResponse,
  MeResponse,
  DeviceApproveResponse,
} from "../../src/lib/api";
import type {
  Agent,
  Capability,
  Friendship,
  Grant,
  Invocation,
  InvokeResponse,
  Visibility,
} from "../../src/lib/relay";

export const APP_BASE_URL = process.env.E2E_APP_URL ?? "http://localhost:8080";
export const RELAY_BASE_URL = process.env.E2E_RELAY_URL ?? "http://localhost:8090";

export class E2EApiError extends Error {
  constructor(
    public readonly url: string,
    public readonly status: number,
    public readonly body: string,
  ) {
    super(`${status} ${url} :: ${body}`);
  }
}

async function jsonRequest<T>(
  url: string,
  init: RequestInit & { token?: string } = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  if (!headers.has("content-type") && init.body) {
    headers.set("content-type", "application/json");
  }
  if (init.token) headers.set("authorization", `Bearer ${init.token}`);

  const res = await fetch(url, { ...init, headers });
  const text = await res.text();
  if (!res.ok) {
    throw new E2EApiError(url, res.status, text || res.statusText);
  }
  if (res.status === 204 || !text) return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new E2EApiError(url, res.status, `non-JSON body: ${text}`);
  }
}

// ─── App service (:8080) ─────────────────────────────────

export function signupWithPassword(args: {
  email: string;
  password: string;
  name: string;
}) {
  return jsonRequest<AuthResponse>(`${APP_BASE_URL}/v1/auth/signup`, {
    method: "POST",
    body: JSON.stringify(args),
  });
}

export function loginWithPassword(args: { email: string; password: string }) {
  return jsonRequest<AuthResponse>(`${APP_BASE_URL}/v1/auth/login`, {
    method: "POST",
    body: JSON.stringify(args),
  });
}

export function getMe(token: string) {
  return jsonRequest<MeResponse>(`${APP_BASE_URL}/v1/me`, { token });
}

export function createApiKey(
  token: string,
  body: { name: string; expires_in_days?: number | null },
) {
  return jsonRequest<CreateApiKeyResponse>(`${APP_BASE_URL}/v1/api-keys`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function listApiKeys(token: string) {
  return jsonRequest<ApiKey[]>(`${APP_BASE_URL}/v1/api-keys`, { token });
}

export function revokeApiKey(token: string, id: string) {
  return jsonRequest<void>(
    `${APP_BASE_URL}/v1/api-keys/${encodeURIComponent(id)}`,
    { method: "DELETE", token },
  );
}

export function revokePairing(
  token: string,
  kind: "device_flow" | "oauth" | "api_key",
  id: string,
) {
  return jsonRequest<void>(
    `${APP_BASE_URL}/v1/pairings/${encodeURIComponent(kind)}/${encodeURIComponent(id)}/revoke`,
    { method: "POST", token },
  );
}

// ─── OAuth device flow ───────────────────────────────────

export interface DeviceAuthResponse {
  device_code: string;
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string;
  verification_uri_qr: string;
  expires_in: number;
  interval: number;
}

export function deviceAuthorize(args: {
  persona?: string;
  agent_slug_hint?: string;
  agent_display_name_hint?: string;
  agent_description_hint?: string;
}) {
  return jsonRequest<DeviceAuthResponse>(
    `${APP_BASE_URL}/oauth/device_authorization`,
    {
      method: "POST",
      body: JSON.stringify({ scope: "relay.full", ...args }),
    },
  );
}

export interface DeviceTokenResponse {
  access_token: string;
  token_type: "Bearer";
  expires_in: number;
  scope: string;
  agent_id: string | null;
  agent_slug: string | null;
  account_slug: string | null;
}

/**
 * Poll RFC 8628 /oauth/token until consent lands or the budget runs
 * out. Form-encoded per RFC 6749.
 */
export async function pollDeviceToken(args: {
  deviceCode: string;
  intervalMs?: number;
  timeoutMs?: number;
}): Promise<DeviceTokenResponse> {
  const interval = args.intervalMs ?? 1_500;
  const deadline = Date.now() + (args.timeoutMs ?? 90_000);
  const body = new URLSearchParams({
    grant_type: "urn:ietf:params:oauth:grant-type:device_code",
    device_code: args.deviceCode,
  }).toString();

  // First request is immediate; subsequent retries wait `interval` ms.
  let firstAttempt = true;
  while (Date.now() < deadline) {
    if (!firstAttempt) {
      await new Promise((r) => setTimeout(r, interval));
    }
    firstAttempt = false;

    const res = await fetch(`${APP_BASE_URL}/oauth/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body,
    });
    const text = await res.text();
    if (res.ok) {
      return JSON.parse(text) as DeviceTokenResponse;
    }
    // RFC 8628 §3.5 pending shapes — keep polling. Anything else is fatal.
    let oauthErr = "";
    try {
      oauthErr = (JSON.parse(text) as { error?: string }).error ?? "";
    } catch {
      // non-JSON; treat as fatal.
    }
    if (oauthErr === "authorization_pending" || oauthErr === "slow_down") {
      continue;
    }
    throw new E2EApiError(`${APP_BASE_URL}/oauth/token`, res.status, text);
  }
  throw new Error(
    `pollDeviceToken: budget exhausted after ${args.timeoutMs ?? 90_000}ms`,
  );
}

export function approveDeviceSession(
  token: string,
  body: {
    user_code: string;
    agent_slug: string;
    agent_display_name: string;
    agent_description?: string;
    agent_visibility?: "private" | "network";
    account_slug?: string;
  },
) {
  return jsonRequest<DeviceApproveResponse>(
    `${APP_BASE_URL}/oauth/device-approve`,
    { method: "POST", token, body: JSON.stringify(body) },
  );
}

// ─── Relay service (:8090) ───────────────────────────────

export function listMyAgents(token: string) {
  return jsonRequest<Agent[]>(`${RELAY_BASE_URL}/v1/agents`, { token });
}

export function createAgent(
  token: string,
  body: {
    account_id: string;
    slug: string;
    display_name: string;
    description?: string;
    visibility?: Visibility;
  },
) {
  return jsonRequest<Agent>(`${RELAY_BASE_URL}/v1/agents`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function deleteAgent(token: string, id: string) {
  return jsonRequest<void>(`${RELAY_BASE_URL}/v1/agents/${encodeURIComponent(id)}`, {
    method: "DELETE",
    token,
  });
}

export function createCapability(
  token: string,
  agentId: string,
  body: {
    name: string;
    description?: string;
    input_schema?: Record<string, unknown>;
    output_schema?: Record<string, unknown>;
    visibility?: Visibility;
  },
) {
  return jsonRequest<Capability>(
    `${RELAY_BASE_URL}/v1/agents/${encodeURIComponent(agentId)}/capabilities`,
    { method: "POST", token, body: JSON.stringify(body) },
  );
}

export function proposeFriendship(
  token: string,
  body: { proposer_agent_id: string; target_agent_id: string; proposer_message?: string | null },
) {
  return jsonRequest<Friendship>(`${RELAY_BASE_URL}/v1/friendships`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function acceptFriendship(token: string, id: string) {
  return jsonRequest<Friendship>(
    `${RELAY_BASE_URL}/v1/friendships/${encodeURIComponent(id)}/accept`,
    { method: "POST", token, body: JSON.stringify({}) },
  );
}

export function createGrant(
  token: string,
  body: { granter_agent_id: string; grantee_agent_id: string; capability_id: string },
) {
  return jsonRequest<Grant>(`${RELAY_BASE_URL}/v1/grants`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function invoke(
  token: string,
  body: { grant_id: string; grantee_agent_id: string; input: unknown },
) {
  return jsonRequest<InvokeResponse>(`${RELAY_BASE_URL}/v1/invoke`, {
    method: "POST",
    token,
    body: JSON.stringify(body),
  });
}

export function pullInbox(token: string, agentId: string, limit = 25) {
  const qs = new URLSearchParams({ agent_id: agentId, limit: String(limit) });
  return jsonRequest<Invocation[]>(
    `${RELAY_BASE_URL}/v1/inbox?${qs.toString()}`,
    { token },
  );
}

export function reportResult(
  token: string,
  invocationId: string,
  body: { status: "succeeded" | "failed"; output?: unknown; error?: string | null },
) {
  return jsonRequest<Invocation>(
    `${RELAY_BASE_URL}/v1/invocations/${encodeURIComponent(invocationId)}/result`,
    { method: "POST", token, body: JSON.stringify(body) },
  );
}
