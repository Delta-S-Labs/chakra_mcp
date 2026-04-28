/**
 * @chakramcp/sdk - typed client for the ChakraMCP relay.
 *
 * Quick start:
 *
 *   import { ChakraMCP } from "@chakramcp/sdk";
 *
 *   const chakra = new ChakraMCP({
 *     apiKey: process.env.CHAKRAMCP_API_KEY!,
 *     // appUrl + relayUrl default to the hosted public network.
 *   });
 *
 *   const me = await chakra.me();
 *   const agents = await chakra.agents.list();
 *
 *   // Synchronous-style invoke: enqueue + poll until terminal.
 *   const result = await chakra.invokeAndWait({
 *     grantId: "…", granteeAgentId: "…", input: { ... },
 *   });
 *
 *   // Granter side: turn one of your agents into a worker.
 *   await chakra.inbox.serve(myAgentId, async (inv) => {
 *     const out = await myAgentLogic(inv.input_preview);
 *     return { status: "succeeded", output: out };
 *   });
 */

export * from "./types.js";
import {
  Agent,
  AgentSummary,
  Capability,
  ChakraMCPError,
  CreateAgentRequest,
  CreateCapabilityRequest,
  CreateGrantRequest,
  FriendshipCounterRequest,
  FriendshipResponseRequest,
  Friendship,
  Grant,
  GrantStatus,
  HandlerResult,
  InvokeRequest,
  InvokeResponse,
  Invocation,
  InvocationStatus,
  MeResponse,
  ProposeFriendshipRequest,
  RevokeGrantRequest,
  TERMINAL_STATUSES,
  UpdateAgentRequest,
} from "./types.js";

export interface ChakraMCPOptions {
  /** API key - `ck_…`. Required. */
  apiKey: string;
  /** Override the chakramcp-app base URL (default: https://chakramcp.com). */
  appUrl?: string;
  /** Override the chakramcp-relay base URL (default: https://relay.chakramcp.com). */
  relayUrl?: string;
  /** Override the global fetch implementation (e.g. for testing). */
  fetch?: typeof fetch;
}

const DEFAULT_APP_URL = "https://chakramcp.com";
const DEFAULT_RELAY_URL = "https://relay.chakramcp.com";

export class ChakraMCP {
  readonly appUrl: string;
  readonly relayUrl: string;
  readonly agents: AgentsClient;
  readonly friendships: FriendshipsClient;
  readonly grants: GrantsClient;
  readonly invocations: InvocationsClient;
  readonly inbox: InboxClient;

  private readonly fetcher: typeof fetch;
  private readonly headers: Record<string, string>;

  constructor(opts: ChakraMCPOptions) {
    if (!opts.apiKey || !opts.apiKey.startsWith("ck_")) {
      throw new Error("ChakraMCP: apiKey must be a `ck_…` API key.");
    }
    this.appUrl = (opts.appUrl ?? DEFAULT_APP_URL).replace(/\/$/, "");
    this.relayUrl = (opts.relayUrl ?? DEFAULT_RELAY_URL).replace(/\/$/, "");
    this.fetcher = opts.fetch ?? fetch;
    this.headers = {
      authorization: `Bearer ${opts.apiKey}`,
      "user-agent": "@chakramcp/sdk",
    };

    this.agents = new AgentsClient(this);
    this.friendships = new FriendshipsClient(this);
    this.grants = new GrantsClient(this);
    this.invocations = new InvocationsClient(this);
    this.inbox = new InboxClient(this);
  }

  /** Current user + memberships. */
  async me(): Promise<MeResponse> {
    return this.appRequest<MeResponse>("GET", "/v1/me");
  }

  /** Discover all network-visible agents on this relay. */
  async network(): Promise<Agent[]> {
    return this.relayRequest<Agent[]>("GET", "/v1/network/agents");
  }

  /**
   * Enqueue an invocation. Returns immediately with the invocation id -
   * use `invokeAndWait` to also poll for the terminal result.
   */
  async invoke(args: InvokeRequest): Promise<InvokeResponse> {
    return this.relayRequest<InvokeResponse>("POST", "/v1/invoke", {
      grant_id: args.grant_id,
      grantee_agent_id: args.grantee_agent_id,
      input: args.input,
    });
  }

  /**
   * Enqueue an invocation and poll until status is terminal.
   *
   * Defaults: poll every 1500ms, time out after 3 minutes. The returned
   * Invocation has the terminal status - caller is expected to inspect
   * `status` and either consume `output_preview` or surface
   * `error_message`.
   */
  async invokeAndWait(
    args: InvokeRequest,
    opts: { intervalMs?: number; timeoutMs?: number; signal?: AbortSignal } = {},
  ): Promise<Invocation> {
    const interval = opts.intervalMs ?? 1500;
    const timeout = opts.timeoutMs ?? 180_000;
    const start = Date.now();

    const enqueued = await this.invoke(args);
    if (TERMINAL_STATUSES.has(enqueued.status as InvocationStatus)) {
      return this.invocations.get(enqueued.invocation_id);
    }

    while (Date.now() - start < timeout) {
      if (opts.signal?.aborted) throw new Error("invokeAndWait aborted");
      await sleep(interval, opts.signal);
      const fresh = await this.invocations.get(enqueued.invocation_id);
      if (TERMINAL_STATUSES.has(fresh.status)) return fresh;
    }
    throw new Error(
      `invokeAndWait timed out after ${timeout}ms - invocation ${enqueued.invocation_id} is still in flight; \`chakra.invocations.get(id)\` later or check the audit log`,
    );
  }

  // ─── Internal request helpers ────────────────────────

  /** @internal */
  appRequest<T>(method: string, path: string, body?: unknown): Promise<T> {
    return this.request<T>(this.appUrl, method, path, body);
  }
  /** @internal */
  relayRequest<T>(method: string, path: string, body?: unknown): Promise<T> {
    return this.request<T>(this.relayUrl, method, path, body);
  }

  private async request<T>(
    baseUrl: string,
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const init: RequestInit = {
      method,
      headers: { ...this.headers, ...(body !== undefined ? { "content-type": "application/json" } : {}) },
      ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
    };
    const res = await this.fetcher(`${baseUrl}${path}`, init);
    if (res.status === 204) return undefined as T;

    const text = await res.text();
    let parsed: unknown = null;
    if (text) {
      try {
        parsed = JSON.parse(text);
      } catch {
        if (!res.ok) {
          throw new ChakraMCPError(res.status, "invalid_response", text);
        }
        throw new ChakraMCPError(res.status, "invalid_response", `non-JSON response: ${text}`);
      }
    }
    if (!res.ok) {
      const env = parsed as { error?: { code?: string; message?: string } } | null;
      throw new ChakraMCPError(
        res.status,
        env?.error?.code ?? "unknown",
        env?.error?.message ?? res.statusText,
      );
    }
    return parsed as T;
  }
}

// ─── Sub-clients ─────────────────────────────────────────

export class AgentsClient {
  constructor(private readonly chakra: ChakraMCP) {}

  list(): Promise<Agent[]> {
    return this.chakra.relayRequest<Agent[]>("GET", "/v1/agents");
  }
  get(id: string): Promise<Agent> {
    return this.chakra.relayRequest<Agent>("GET", `/v1/agents/${encodeURIComponent(id)}`);
  }
  create(body: CreateAgentRequest): Promise<Agent> {
    return this.chakra.relayRequest<Agent>("POST", "/v1/agents", body);
  }
  update(id: string, body: UpdateAgentRequest): Promise<Agent> {
    return this.chakra.relayRequest<Agent>("PATCH", `/v1/agents/${encodeURIComponent(id)}`, body);
  }
  delete(id: string): Promise<void> {
    return this.chakra.relayRequest<void>("DELETE", `/v1/agents/${encodeURIComponent(id)}`);
  }

  capabilities = {
    list: (agentId: string): Promise<Capability[]> =>
      this.chakra.relayRequest("GET", `/v1/agents/${encodeURIComponent(agentId)}/capabilities`),
    create: (agentId: string, body: CreateCapabilityRequest): Promise<Capability> =>
      this.chakra.relayRequest(
        "POST",
        `/v1/agents/${encodeURIComponent(agentId)}/capabilities`,
        body,
      ),
    delete: (agentId: string, capabilityId: string): Promise<void> =>
      this.chakra.relayRequest(
        "DELETE",
        `/v1/agents/${encodeURIComponent(agentId)}/capabilities/${encodeURIComponent(capabilityId)}`,
      ),
  };
}

export class FriendshipsClient {
  constructor(private readonly chakra: ChakraMCP) {}

  list(opts: { direction?: "all" | "outbound" | "inbound" } = {}): Promise<Friendship[]> {
    const qs = opts.direction ? `?direction=${opts.direction}` : "";
    return this.chakra.relayRequest("GET", `/v1/friendships${qs}`);
  }
  get(id: string): Promise<Friendship> {
    return this.chakra.relayRequest("GET", `/v1/friendships/${encodeURIComponent(id)}`);
  }
  propose(body: ProposeFriendshipRequest): Promise<Friendship> {
    return this.chakra.relayRequest("POST", "/v1/friendships", body);
  }
  accept(id: string, body: FriendshipResponseRequest = {}): Promise<Friendship> {
    return this.chakra.relayRequest("POST", `/v1/friendships/${encodeURIComponent(id)}/accept`, body);
  }
  reject(id: string, body: FriendshipResponseRequest = {}): Promise<Friendship> {
    return this.chakra.relayRequest("POST", `/v1/friendships/${encodeURIComponent(id)}/reject`, body);
  }
  counter(id: string, body: FriendshipCounterRequest): Promise<Friendship> {
    return this.chakra.relayRequest("POST", `/v1/friendships/${encodeURIComponent(id)}/counter`, body);
  }
  cancel(id: string): Promise<Friendship> {
    return this.chakra.relayRequest("POST", `/v1/friendships/${encodeURIComponent(id)}/cancel`, {});
  }
}

export class GrantsClient {
  constructor(private readonly chakra: ChakraMCP) {}

  list(opts: { direction?: "all" | "outbound" | "inbound"; status?: GrantStatus } = {}): Promise<Grant[]> {
    const qs = new URLSearchParams();
    if (opts.direction) qs.set("direction", opts.direction);
    if (opts.status) qs.set("status", opts.status);
    const q = qs.toString();
    return this.chakra.relayRequest("GET", `/v1/grants${q ? `?${q}` : ""}`);
  }
  get(id: string): Promise<Grant> {
    return this.chakra.relayRequest("GET", `/v1/grants/${encodeURIComponent(id)}`);
  }
  create(body: CreateGrantRequest): Promise<Grant> {
    return this.chakra.relayRequest("POST", "/v1/grants", body);
  }
  revoke(id: string, body: RevokeGrantRequest = {}): Promise<Grant> {
    return this.chakra.relayRequest("POST", `/v1/grants/${encodeURIComponent(id)}/revoke`, body);
  }
}

export class InvocationsClient {
  constructor(private readonly chakra: ChakraMCP) {}

  list(opts: {
    direction?: "all" | "outbound" | "inbound";
    agentId?: string;
    status?: InvocationStatus;
  } = {}): Promise<Invocation[]> {
    const qs = new URLSearchParams();
    if (opts.direction) qs.set("direction", opts.direction);
    if (opts.agentId) qs.set("agent_id", opts.agentId);
    if (opts.status) qs.set("status", opts.status);
    const q = qs.toString();
    return this.chakra.relayRequest("GET", `/v1/invocations${q ? `?${q}` : ""}`);
  }
  get(id: string): Promise<Invocation> {
    return this.chakra.relayRequest("GET", `/v1/invocations/${encodeURIComponent(id)}`);
  }
}

export interface ServeOptions {
  /** How often to poll when the inbox is empty. Default 2000ms. */
  pollIntervalMs?: number;
  /** Max rows to claim per poll. Default 25. */
  batchSize?: number;
  /** Cancel the loop. */
  signal?: AbortSignal;
  /** Hook for observability. */
  onError?: (err: unknown, invocation?: Invocation) => void;
}

export class InboxClient {
  constructor(private readonly chakra: ChakraMCP) {}

  /** Atomically claim the oldest pending invocations targeting an agent you own. */
  async pull(agentId: string, opts: { limit?: number } = {}): Promise<Invocation[]> {
    const qs = new URLSearchParams({ agent_id: agentId });
    if (opts.limit) qs.set("limit", String(opts.limit));
    return this.chakra.relayRequest("GET", `/v1/inbox?${qs.toString()}`);
  }

  /** Report the result for an in_progress invocation you previously claimed. */
  async respond(
    invocationId: string,
    body:
      | { status: "succeeded"; output: unknown }
      | { status: "failed"; error?: string },
  ): Promise<Invocation> {
    return this.chakra.relayRequest(
      "POST",
      `/v1/invocations/${encodeURIComponent(invocationId)}/result`,
      body,
    );
  }

  /**
   * Long-running auto-loop: pull → run handler → respond, forever.
   *
   * The handler returns either `{ status: 'succeeded', output }` or
   * `{ status: 'failed', error }`. Throws are caught and reported as
   * failed; the loop keeps running.
   */
  async serve(
    agentId: string,
    handler: (inv: Invocation) => Promise<HandlerResult>,
    opts: ServeOptions = {},
  ): Promise<void> {
    const interval = opts.pollIntervalMs ?? 2000;
    const limit = opts.batchSize ?? 25;
    while (!opts.signal?.aborted) {
      let batch: Invocation[];
      try {
        batch = await this.pull(agentId, { limit });
      } catch (err) {
        opts.onError?.(err);
        await sleep(interval, opts.signal);
        continue;
      }
      if (batch.length === 0) {
        await sleep(interval, opts.signal);
        continue;
      }
      // Process in parallel - invocations are independent.
      await Promise.all(
        batch.map(async (inv) => {
          try {
            const result = await handler(inv);
            await this.respond(inv.id, result);
          } catch (err) {
            opts.onError?.(err, inv);
            try {
              await this.respond(inv.id, {
                status: "failed",
                error: err instanceof Error ? err.message : String(err),
              });
            } catch (innerErr) {
              opts.onError?.(innerErr, inv);
            }
          }
        }),
      );
    }
  }
}

/**
 * Sleep for `ms` milliseconds. Resolves early (does NOT reject) on
 * abort - callers check `signal.aborted` themselves on the next
 * iteration, which is the pattern serve() and invokeAndWait() use.
 */
function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) {
      resolve();
      return;
    }
    const t = setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        clearTimeout(t);
        resolve();
      },
      { once: true },
    );
  });
}

// Re-export the AgentSummary too for downstream consumers building DTOs.
export type { AgentSummary };
