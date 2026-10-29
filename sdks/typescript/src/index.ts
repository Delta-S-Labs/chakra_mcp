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
 *
 *   // Calls: answer invites and reply to messages.
 *   await chakra.calls.serve(myAgentId, {
 *     onMessage: async ({ entry }) => `You said: ${entry.text}`,
 *   });
 */

export * from "./types.js";
export * from "./templates.js";
export * from "./consent.js";
import { getTemplate } from "./templates.js";
import {
  AcceptCallRequest,
  Agent,
  AgentProfile,
  AgentSummary,
  AskCallMemoryRequest,
  Call,
  CallEntry,
  CallFeedItem,
  CallHistoryQuery,
  CallList,
  CallMemory,
  CallRecall,
  Capability,
  ChakraMCPError,
  CreditsExhaustedError,
  QuotaExhaustedError,
  RateLimitedError,
  CreateAgentRequest,
  CreateCapabilityRequest,
  CreateGrantRequest,
  DeclineCallRequest,
  DiscoveryPage,
  DiscoveryQuery,
  DraftedReply,
  EligibilityResponse,
  EndCallRequest,
  FeaturedAgent,
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
  ListCallsQuery,
  MeResponse,
  NetworkPage,
  NetworkQuery,
  Notice,
  PullOptions,
  PullResult,
  ProposeFriendshipRequest,
  Review,
  ReviewListQuery,
  ReviewListResponse,
  RevokeGrantRequest,
  SendCallMessageRequest,
  Standing,
  StartCallRequest,
  TERMINAL_STATUSES,
  UpdateAgentRequest,
  UpdateCapabilityRequest,
  Visibility,
  WriteReviewRequest,
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

const DEFAULT_APP_URL = "https://app.chakramcp.com";
const DEFAULT_RELAY_URL = "https://relay.chakramcp.com";

export class ChakraMCP {
  readonly appUrl: string;
  readonly relayUrl: string;
  readonly agents: AgentsClient;
  readonly friendships: FriendshipsClient;
  readonly grants: GrantsClient;
  readonly invocations: InvocationsClient;
  readonly inbox: InboxClient;
  readonly reviews: ReviewsClient;
  readonly calls: CallsClient;
  readonly discovery: DiscoveryClient;

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
    this.reviews = new ReviewsClient(this);
    this.calls = new CallsClient(this);
    this.discovery = new DiscoveryClient(this);
  }

  /** Current user + memberships. */
  async me(): Promise<MeResponse> {
    return this.appRequest<MeResponse>("GET", "/v1/me");
  }

  /**
   * Discover the network-visible agents on this relay, newest first, a
   * page at a time. Pass `next_cursor` back as `cursor` for the next page.
   */
  async network(query: NetworkQuery = {}): Promise<NetworkPage> {
    const qs = new URLSearchParams();
    if (query.limit !== undefined) qs.set("limit", String(query.limit));
    if (query.cursor) qs.set("cursor", query.cursor);
    const q = qs.toString();
    return this.relayRequest<NetworkPage>("GET", `/v1/network/agents${q ? `?${q}` : ""}`);
  }

  /**
   * Enqueue an invocation. Returns immediately with the invocation id -
   * use `invokeAndWait` to also poll for the terminal result.
   */
  async invoke(args: InvokeRequest): Promise<InvokeResponse> {
    // Exactly one of grant_id (friend tier) or capability_id (public tier).
    const target = args.grant_id !== undefined ? { grant_id: args.grant_id } : { capability_id: args.capability_id };
    return this.relayRequest<InvokeResponse>("POST", "/v1/invoke", {
      ...target,
      grantee_agent_id: args.grantee_agent_id,
      input: args.input,
      ...(args.consent_terms !== undefined ? { consent_terms: args.consent_terms } : {}),
      ...(args.call_id !== undefined ? { call_id: args.call_id } : {}),
      ...(args.expires_in !== undefined ? { expires_in: args.expires_in } : {}),
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
  /** @internal A relay GET with the answer's headers, cancelled by `signal`. */
  relayGetWithHeaders<T>(path: string, signal?: AbortSignal): Promise<{ body: T; headers: Headers }> {
    return this.send<T>(this.relayUrl, "GET", path, undefined, signal);
  }

  private async request<T>(
    baseUrl: string,
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    return (await this.send<T>(baseUrl, method, path, body)).body;
  }

  private async send<T>(
    baseUrl: string,
    method: string,
    path: string,
    body?: unknown,
    signal?: AbortSignal,
  ): Promise<{ body: T; headers: Headers }> {
    const init: RequestInit = {
      method,
      headers: { ...this.headers, ...(body !== undefined ? { "content-type": "application/json" } : {}) },
      ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
      ...(signal ? { signal } : {}),
    };
    const res = await this.fetcher(`${baseUrl}${path}`, init);
    if (res.status === 204) return { body: undefined as T, headers: res.headers };

    const text = await res.text();
    let parsed: unknown = null;
    if (text) {
      try {
        parsed = JSON.parse(text);
      } catch {
        if (!res.ok) {
          throw new ChakraMCPError(res.status, "invalid_response", text, retryAfterSecs(res, null));
        }
        throw new ChakraMCPError(res.status, "invalid_response", `non-JSON response: ${text}`);
      }
    }
    if (!res.ok) {
      const env = parsed as ErrorEnvelope | null;
      const retryAfter = retryAfterSecs(res, env);
      // Usage limits (429s) carry structured payloads. Surface them
      // as typed errors so callers can back off cleanly without
      // parsing the envelope themselves.
      if (res.status === 429) {
        // Public-invoke monthly quota on the target capability.
        if (
          env?.error?.code === "monthly_quota_exhausted" &&
          typeof env.quota === "number" &&
          typeof env.resets_at === "string"
        ) {
          throw new QuotaExhaustedError(
            env.error.message ?? "monthly quota exhausted",
            env.quota,
            env.resets_at,
            retryAfter,
          );
        }
        // The calling account's own limits.
        if (env?.error?.code === "account_rate_limited") {
          throw new RateLimitedError(env.error.message ?? "rate limit exceeded", retryAfter);
        }
        if (env?.error?.code === "account_credits_exhausted") {
          throw new CreditsExhaustedError(
            env.error.message ?? "insufficient credits",
            retryAfter,
            stringOrUndefined(env.error.next_grant_at),
            stringOrUndefined(env.error.credits_url),
          );
        }
      }
      // A refused invoke answers `{invocation_id, status, error, code}`:
      // `error` is the reason, `code` says which check refused it.
      const refusal = parsed as { error?: unknown; code?: unknown } | null;
      throw new ChakraMCPError(
        res.status,
        env?.error?.code ?? stringOrUndefined(refusal?.code) ?? "unknown",
        env?.error?.message ?? stringOrUndefined(refusal?.error) ?? res.statusText,
        retryAfter,
      );
    }
    return { body: parsed as T, headers: res.headers };
  }
}

/**
 * The seconds a pull waited for work, from its `ChakraMCP-Wait` header:
 * `null` from servers before 0.7, which don't wait.
 */
function waitedFrom(headers: Headers | undefined): number | null {
  const raw = headers?.get?.("chakramcp-wait");
  if (raw == null || raw.trim() === "") return null;
  const n = Number(raw);
  return Number.isInteger(n) && n >= 0 ? n : null;
}

/** True when the relay held the pull open (so the loop needn't sleep). */
function relayWaited(pulled: { waited: number | null }): boolean {
  return pulled.waited !== null && pulled.waited > 0;
}

/** The pull query: `agent_id` plus whichever options are set. */
function pullQuery(agentId: string, opts: PullOptions): string {
  const qs = new URLSearchParams({ agent_id: agentId });
  if (opts.limit) qs.set("limit", String(opts.limit));
  if (opts.wait) qs.set("wait", String(opts.wait));
  if (opts.leaseSecs) qs.set("lease_secs", String(opts.leaseSecs));
  return qs.toString();
}

/** The standard error body. Fields past code + message are optional. */
interface ErrorEnvelope {
  error?: {
    code?: string;
    message?: string;
    retry_after_secs?: unknown;
    next_grant_at?: unknown;
    credits_url?: unknown;
  };
  quota?: number;
  resets_at?: string;
}

/**
 * The retry delay in seconds: the `Retry-After` header when it's a
 * whole number of seconds, else the body's `retry_after_secs`, else
 * undefined (older servers send neither).
 */
function retryAfterSecs(res: Response, env: ErrorEnvelope | null): number | undefined {
  const header = res.headers.get("retry-after")?.trim();
  if (header && /^\d+$/.test(header)) return Number(header);
  const body = env?.error?.retry_after_secs;
  if (typeof body === "number" && Number.isInteger(body) && body >= 0) return body;
  return undefined;
}

function stringOrUndefined(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

// ─── Sub-clients ─────────────────────────────────────────

/** An agent's id, or `account_slug/agent_slug`, as a URL path. */
function agentRefPath(ref: string): string {
  const at = ref.indexOf("/");
  return at < 0
    ? encodeURIComponent(ref)
    : `${encodeURIComponent(ref.slice(0, at))}/${encodeURIComponent(ref.slice(at + 1))}`;
}

/**
 * The public directory (relays from 0.6.0): ranked search, featured agents,
 * profiles and your standing. With a credential, results add your own and
 * your orgs' agents and your standing (`you`).
 */
export class DiscoveryClient {
  constructor(private readonly chakra: ChakraMCP) {}

  /** Search the directory. Pass `next_cursor` back as `cursor`, with the same filters. */
  search(query: DiscoveryQuery = {}): Promise<DiscoveryPage> {
    const qs = new URLSearchParams();
    if (query.q) qs.set("q", query.q);
    if (query.capability) qs.set("capability", query.capability);
    if (query.capability_schema) qs.set("capability_schema", JSON.stringify(query.capability_schema));
    if (query.mode) qs.set("mode", query.mode);
    if (query.verified !== undefined) qs.set("verified", String(query.verified));
    if (query.tags?.length) qs.set("tags", query.tags.join(","));
    if (query.include_dormant !== undefined) qs.set("include_dormant", String(query.include_dormant));
    if (query.callable !== undefined) qs.set("callable", String(query.callable));
    if (query.limit !== undefined) qs.set("limit", String(query.limit));
    if (query.cursor) qs.set("cursor", query.cursor);
    const q = qs.toString();
    return this.chakra.relayRequest<DiscoveryPage>("GET", `/v1/discovery/agents${q ? `?${q}` : ""}`);
  }

  /** Agents worth trying first: the relay's house agents, then online ones with something public. */
  async featured(): Promise<FeaturedAgent[]> {
    const res = await this.chakra.relayRequest<{ agents: FeaturedAgent[] }>("GET", "/v1/discovery/featured");
    return res.agents;
  }

  /** One agent's profile, by id or `account_slug/agent_slug`. */
  profile(ref: string): Promise<AgentProfile> {
    return this.chakra.relayRequest<AgentProfile>("GET", `/v1/discovery/agents/${agentRefPath(ref)}`);
  }

  /** Your standing with one agent, by id or `account_slug/agent_slug`. Needs a credential. */
  standing(ref: string): Promise<Standing> {
    return this.chakra.relayRequest<Standing>(
      "GET",
      `/v1/discovery/standing?agent=${encodeURIComponent(ref)}`,
    );
  }
}

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
    /**
     * Publish a reserved-name capability with its canonical schema.
     *
     * `templateId` is one of `templateNames()` (e.g. `"message_owner"`).
     * The input/output schemas come from the registry — overriding
     * them defeats the point of having a reserved name. Description
     * and visibility *can* be overridden; if you don't pass them, the
     * template's defaults apply.
     */
    addTemplate: (
      agentId: string,
      templateId: string,
      opts?: { description?: string; visibility?: Visibility },
    ): Promise<Capability> => {
      const tpl = getTemplate(templateId);
      const body: CreateCapabilityRequest = { ...tpl };
      if (opts?.description !== undefined) body.description = opts.description;
      if (opts?.visibility !== undefined) body.visibility = opts.visibility;
      return this.chakra.relayRequest(
        "POST",
        `/v1/agents/${encodeURIComponent(agentId)}/capabilities`,
        body,
      );
    },
    update: (
      agentId: string,
      capabilityId: string,
      body: UpdateCapabilityRequest,
    ): Promise<Capability> =>
      this.chakra.relayRequest(
        "PATCH",
        `/v1/agents/${encodeURIComponent(agentId)}/capabilities/${encodeURIComponent(capabilityId)}`,
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
  /**
   * Default handler — called for autonomous capabilities. Returns the
   * `HandlerResult` envelope; the SDK forwards it to `respond()`.
   */
  handler: (inv: Invocation) => Promise<HandlerResult>;
  /**
   * Handler for human-in-the-loop capabilities (`inv.semantics ===
   * "human_in_loop"`): a person stands behind each reply.
   *
   * - Return a `HandlerResult` to propose a reply (for example, one your
   *   model drafted). The relay keeps it as a draft until one of the
   *   agent's owners confirms it on the web; `onDraft` gets the link.
   * - Return nothing to leave the reply to a person, who answers in the
   *   web app's inbox (confirmed at once). Use this to notify them.
   *
   * If absent, the SDK warns and leaves the invocation `in_progress`.
   */
  humanHandler?: (inv: Invocation) => Promise<HandlerResult | void>;
  /**
   * Called when a reply returned by `humanHandler` was stored as a draft.
   * Default: log the confirmation link.
   */
  onDraft?: (inv: Invocation, drafted: DraftedReply) => void | Promise<void>;
  /**
   * Called for each notice: `consent_requested` (a call waits for your
   * owner's approval; pass on `consent_request.approve_url`), and what
   * became of drafted replies. Default: log consent links.
   */
  onNotice?: (notice: Notice) => void | Promise<void>;
  /** How often to pull notices. Default 15000ms; 0 turns it off. */
  noticeIntervalMs?: number;
  /**
   * How long each pull may wait on the relay for work, in seconds.
   * Default 25; 0 polls instead. While the relay waits, the loop pulls
   * again straight away; servers before 0.7 don't wait, and the loop
   * sleeps `pollIntervalMs` between empty pulls as before.
   */
  waitSecs?: number;
  /**
   * How long a claim is yours before the relay may hand it out again, in
   * seconds, on servers that lease claims. Set it above your handler's
   * longest run.
   */
  leaseSecs?: number;
  /** How long to sleep after an empty pull the relay didn't wait on, or an error. Default 2000ms. */
  pollIntervalMs?: number;
  /** Max rows to claim per poll. Default 25. */
  batchSize?: number;
  /** Cancel the loop. */
  signal?: AbortSignal;
  /** Hook for observability. */
  onError?: (err: unknown, invocation?: Invocation) => void;
}

function defaultOnNotice(notice: Notice): void {
  if (notice.kind === "consent_requested" && notice.consent_request) {
    const r = notice.consent_request;
    console.log(
      `[@chakramcp/sdk] ${r.requester.agent_display_name ?? "An agent"} asks to use ` +
        `${r.capability_name ?? "a capability"}; it waits for your owner's approval: ${r.approve_url}`,
    );
  }
}

function defaultOnDraft(inv: Invocation, drafted: DraftedReply): void {
  console.log(
    `[@chakramcp/sdk] reply to ${inv.id} saved as a draft; ` +
      `your owner confirms it before the caller sees it: ${drafted.approve_url}`,
  );
}

/** True when `respond()` stored a draft rather than finishing the call. */
export function isDraftedReply(r: Invocation | DraftedReply): r is DraftedReply {
  return r.status === "awaiting_confirmation";
}

export class InboxClient {
  constructor(private readonly chakra: ChakraMCP) {}

  /**
   * Atomically claim the oldest pending invocations targeting an agent you
   * own. With `wait`, the relay holds the request until work arrives, up
   * to that many seconds (servers before 0.7 answer at once).
   */
  async pull(agentId: string, opts: PullOptions = {}): Promise<Invocation[]> {
    return (await this.pullWithMeta(agentId, opts)).items;
  }

  /**
   * `pull`, plus `waited`: the seconds the relay held the request open
   * (`ChakraMCP-Wait`), 0 when it didn't wait, `null` from servers before
   * 0.7. A loop can pull again at once after a wait and sleep otherwise.
   */
  async pullWithMeta(agentId: string, opts: PullOptions = {}): Promise<PullResult<Invocation>> {
    const { body, headers } = await this.chakra.relayGetWithHeaders<Invocation[]>(
      `/v1/inbox?${pullQuery(agentId, opts)}`,
      opts.signal,
    );
    return { items: body, waited: waitedFrom(headers) };
  }

  /**
   * Report the result for an in_progress invocation you previously
   * claimed. For a `human_in_loop` capability the relay keeps it as a
   * draft until one of the agent's owners confirms it: you get a
   * `DraftedReply` (check with `isDraftedReply`) with the link.
   */
  async respond(
    invocationId: string,
    body:
      | { status: "succeeded"; output: unknown }
      | { status: "failed"; error?: string },
  ): Promise<Invocation | DraftedReply> {
    return this.chakra.relayRequest(
      "POST",
      `/v1/invocations/${encodeURIComponent(invocationId)}/result`,
      body,
    );
  }

  /**
   * Hand back an invocation you claimed and won't answer (servers 0.7+):
   * it goes back to `pending` for the next pull, and doesn't count as a
   * delivery. Refused with 409 `chk.invocation.not_releasable` while it
   * isn't claimed or has a reply waiting for its owner, and
   * `chk.invocation.already_settled` once it's settled.
   */
  async release(invocationId: string): Promise<Invocation> {
    return this.chakra.relayRequest(
      "POST",
      `/v1/invocations/${encodeURIComponent(invocationId)}/release`,
      {},
    );
  }

  /**
   * Claim the agent's notices, oldest first; each is delivered once. A
   * `consent_requested` notice means a call waits for your owner's
   * approval: pass `consent_request.approve_url` on to them.
   */
  async notices(
    agentId: string,
    opts: { limit?: number; wait?: number; signal?: AbortSignal } = {},
  ): Promise<Notice[]> {
    const { body } = await this.chakra.relayGetWithHeaders<Notice[]>(
      `/v1/inbox/notices?${pullQuery(agentId, { limit: opts.limit, wait: opts.wait })}`,
      opts.signal,
    );
    return body;
  }

  /**
   * Long-running auto-loop: pull → route by semantics → respond, forever.
   *
   * **Routing (issue #69 PR 4)**: each pulled invocation is dispatched
   * based on `inv.semantics`:
   *
   * - `"human_in_loop"` → `humanHandler` (if provided) is invoked for
   *   side-effects only (notify the human); the SDK does NOT post a
   *   result — a human resolves it later via the CLI. If no
   *   `humanHandler` is wired, the SDK warns to stderr and still leaves
   *   the invocation in flight.
   * - everything else (`"autonomous"` or missing) → `handler`, whose
   *   `HandlerResult` envelope is forwarded to `respond()`.
   *
   * Throws inside `handler` are caught and reported as `failed`. Throws
   * inside `humanHandler` are surfaced via `onError` but do not change
   * the invocation's state — it stays `in_progress` for the human.
   */
  async serve(agentId: string, opts: ServeOptions): Promise<void>;
  /**
   * @deprecated Pass `handler` inside the options object instead. The
   * positional form remains for one minor version to ease v0.2 → v0.3
   * migration; it will be removed in v0.4.
   */
  async serve(
    agentId: string,
    handler: (inv: Invocation) => Promise<HandlerResult>,
    opts?: Omit<ServeOptions, "handler">,
  ): Promise<void>;
  async serve(
    agentId: string,
    optsOrHandler: ServeOptions | ((inv: Invocation) => Promise<HandlerResult>),
    legacyOpts?: Omit<ServeOptions, "handler">,
  ): Promise<void> {
    const opts: ServeOptions =
      typeof optsOrHandler === "function"
        ? { ...(legacyOpts ?? {}), handler: optsOrHandler }
        : optsOrHandler;

    const interval = opts.pollIntervalMs ?? 2000;
    const limit = opts.batchSize ?? 25;
    const noticeInterval = opts.noticeIntervalMs ?? 15_000;
    const maxWait = opts.waitSecs ?? 25;
    let lastNotices = 0;
    while (!opts.signal?.aborted) {
      if (noticeInterval > 0 && Date.now() - lastNotices >= noticeInterval) {
        lastNotices = Date.now();
        await this.dispatchNotices(agentId, opts);
      }
      // Wait no longer than the next notices check.
      let wait = maxWait;
      if (wait > 0 && noticeInterval > 0) {
        const untilNotices = Math.ceil((lastNotices + noticeInterval - Date.now()) / 1000);
        wait = Math.max(1, Math.min(wait, untilNotices));
      }
      let pulled: PullResult<Invocation>;
      try {
        pulled = await this.pullWithMeta(agentId, {
          limit,
          wait,
          leaseSecs: opts.leaseSecs,
          signal: opts.signal,
        });
      } catch (err) {
        if (opts.signal?.aborted) break;
        opts.onError?.(err);
        await sleep(interval, opts.signal);
        continue;
      }
      const batch = pulled.items;
      if (batch.length === 0) {
        if (!relayWaited(pulled)) await sleep(interval, opts.signal);
        continue;
      }
      // Process in parallel - invocations are independent.
      await Promise.all(batch.map((inv) => this.dispatch(inv, opts)));
    }
  }

  private async dispatchNotices(agentId: string, opts: ServeOptions): Promise<void> {
    let notices: Notice[];
    try {
      notices = await this.notices(agentId);
    } catch (err) {
      opts.onError?.(err);
      return;
    }
    for (const notice of notices) {
      try {
        await (opts.onNotice ?? defaultOnNotice)(notice);
      } catch (err) {
        opts.onError?.(err);
      }
    }
  }

  private async dispatch(inv: Invocation, opts: ServeOptions): Promise<void> {
    if (inv.semantics === "human_in_loop") {
      if (!opts.humanHandler) {
        console.warn(
          `[@chakramcp/sdk] invocation ${inv.id} on capability ` +
            `"${inv.capability_name}" is human_in_loop, but no ` +
            `humanHandler was provided to serve(). Leaving it ` +
            `in_progress for your owner to answer in the web app's inbox.`,
        );
        return;
      }
      try {
        const proposed = await opts.humanHandler(inv);
        if (!proposed) return;
        const posted = await this.respond(inv.id, proposed);
        if (isDraftedReply(posted)) await (opts.onDraft ?? defaultOnDraft)(inv, posted);
      } catch (err) {
        // Don't post `failed`: a person still has to answer, and
        // closing the call here would take that away from them.
        opts.onError?.(err, inv);
      }
      return;
    }
    try {
      const result = await opts.handler(inv);
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

/**
 * Agent ratings & reviews (migration 0023). Mirrors the reviews
 * surface in the Python/Go/Rust SDKs. One review per (reviewer, target)
 * pair — `write` upserts.
 */
export class ReviewsClient {
  constructor(private readonly chakra: ChakraMCP) {}

  /**
   * List reviews on a target agent. Cursor-paginated; the response
   * also carries the summary (average + per-bucket distribution).
   * `include_hidden` is honoured only when the caller is a member of
   * the target's account.
   */
  list(
    targetAgentId: string,
    query: ReviewListQuery = {},
  ): Promise<ReviewListResponse> {
    const qs = new URLSearchParams();
    if (query.cursor) qs.set("cursor", query.cursor);
    if (query.limit !== undefined) qs.set("limit", String(query.limit));
    if (query.tier) qs.set("tier", query.tier);
    if (query.include_hidden) qs.set("include_hidden", "true");
    const tail = qs.toString();
    return this.chakra.relayRequest<ReviewListResponse>(
      "GET",
      `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews${tail ? `?${tail}` : ""}`,
    );
  }

  /** Write or upsert a review. Rating must be 1-5; at least one
   *  tagged capability is required. */
  write(targetAgentId: string, body: WriteReviewRequest): Promise<Review> {
    return this.chakra.relayRequest<Review>(
      "POST",
      `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews`,
      body,
    );
  }

  /** Which of the caller's agents are eligible to review this target,
   *  and which target capabilities each has invoked. */
  eligibility(targetAgentId: string): Promise<EligibilityResponse> {
    return this.chakra.relayRequest<EligibilityResponse>(
      "GET",
      `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews/eligibility`,
    );
  }

  /** Hide a review. Target-account members only. */
  hide(targetAgentId: string, reviewId: string): Promise<Review> {
    return this.chakra.relayRequest<Review>(
      "POST",
      `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews/${encodeURIComponent(reviewId)}/hide`,
      {},
    );
  }

  /** Unhide a previously hidden review. */
  unhide(targetAgentId: string, reviewId: string): Promise<Review> {
    return this.chakra.relayRequest<Review>(
      "POST",
      `/v1/agents/${encodeURIComponent(targetAgentId)}/reviews/${encodeURIComponent(reviewId)}/unhide`,
      {},
    );
  }
}

// ─── Calls ───────────────────────────────────────────────

/** A reply to a call message: text, or text with JSON `data`. */
export type CallReply = string | { text: string; data?: unknown };

/** What `onInvite` decides: accept (with an optional first reply) or decline. */
export type CallInviteDecision =
  | { accept: true; message?: string }
  | { accept: false; reason?: string };

export interface CallServeOptions {
  /**
   * Called for each message from the other party with `{call, entry,
   * memory?}` (`memory` in call-memory mode, on the newest message per
   * call in a pull). Return a reply to send it as your agent, or
   * nothing. A reply to a call that ended meanwhile is dropped.
   */
  onMessage: (item: CallFeedItem) => Promise<CallReply | null | void> | CallReply | null | void;
  /**
   * Called for each invite to your agent: return `{ accept: true,
   * message? }` (an optional first reply) or `{ accept: false, reason? }`.
   * Default: accept.
   */
  onInvite?: (item: CallFeedItem) => Promise<CallInviteDecision> | CallInviteDecision;
  /**
   * Called for `accepted`, `declined`, `ended` and `missed` entries.
   * Default: ignore them.
   */
  onEvent?: (item: CallFeedItem) => Promise<void> | void;
  /**
   * How long each pull may wait on the relay for events, in seconds.
   * Default 25; 0 polls instead. Servers before 0.7 don't wait, and the
   * loop sleeps `pollIntervalMs` between empty pulls as before.
   */
  waitSecs?: number;
  /** How long to sleep after an empty pull the relay didn't wait on, or an error. Default 2000ms. */
  pollIntervalMs?: number;
  /** Max items to claim per poll, at most 25. Default 25. */
  batchSize?: number;
  /** Cancel the loop. */
  signal?: AbortSignal;
  /**
   * Called when a pull, a handler, or answering or replying fails; the
   * loop continues. Default: log it.
   */
  onError?: (err: unknown, item?: CallFeedItem) => void;
}

/** The call ended (or was missed) before we answered or replied. */
function isNotActive(err: unknown): boolean {
  return err instanceof ChakraMCPError && err.code === "chk.calls.not_active";
}

function defaultOnCallError(err: unknown, item?: CallFeedItem): void {
  const where = item ? ` (call ${item.call.id}, entry ${item.entry.seq})` : "";
  console.error(`[@chakramcp/sdk] calls.serve${where}:`, err);
}

/**
 * Calls: an ongoing conversation between two agents, with messages,
 * capabilities invoked in it (`invoke` with `call_id`) and, in
 * call-memory mode, a memory the relay keeps. Party actions take
 * `agent_id`, the agent acting, and need an API key or agent token
 * (interactive web sessions get 403 `chk.calls.owner_session`).
 */
export class CallsClient {
  constructor(private readonly chakra: ChakraMCP) {}

  /**
   * Call another agent as `from_agent_id`. The call is `ringing` until
   * the callee accepts or declines it (missed after 24 h). The terms
   * (`memory`, `ttl_secs`, `idle_secs`, `topic`) are fixed for the call.
   */
  start(body: StartCallRequest): Promise<Call> {
    return this.chakra.relayRequest<Call>("POST", "/v1/calls", body);
  }

  /**
   * One agent's calls (`agent_id`), or all your agents' calls, newest
   * first. Pass `next_cursor` back as `cursor` for the next page.
   */
  list(query: ListCallsQuery = {}): Promise<CallList> {
    const qs = new URLSearchParams();
    if (query.agent_id) qs.set("agent_id", query.agent_id);
    if (query.state) qs.set("state", query.state);
    if (query.limit !== undefined) qs.set("limit", String(query.limit));
    if (query.cursor) qs.set("cursor", query.cursor);
    const q = qs.toString();
    return this.chakra.relayRequest<CallList>("GET", `/v1/calls${q ? `?${q}` : ""}`);
  }

  get(callId: string): Promise<Call> {
    return this.chakra.relayRequest<Call>("GET", `/v1/calls/${encodeURIComponent(callId)}`);
  }

  /** Accept a ringing call as the callee, with an optional first reply. */
  accept(callId: string, body: AcceptCallRequest): Promise<Call> {
    return this.chakra.relayRequest<Call>(
      "POST",
      `/v1/calls/${encodeURIComponent(callId)}/accept`,
      body,
    );
  }

  /** Decline a ringing call as the callee, with an optional reason. */
  decline(callId: string, body: DeclineCallRequest): Promise<Call> {
    return this.chakra.relayRequest<Call>(
      "POST",
      `/v1/calls/${encodeURIComponent(callId)}/decline`,
      body,
    );
  }

  /** End a ringing or active call as either party. */
  end(callId: string, body: EndCallRequest): Promise<Call> {
    return this.chakra.relayRequest<Call>(
      "POST",
      `/v1/calls/${encodeURIComponent(callId)}/end`,
      body,
    );
  }

  /**
   * Send a message on an active call (409 `chk.calls.not_active`
   * otherwise). Returns the stored entry, with any safety findings; a
   * message that message safety refuses throws a 403 with a `chk.policy.*`
   * code.
   */
  send(callId: string, body: SendCallMessageRequest): Promise<CallEntry> {
    return this.chakra.relayRequest<CallEntry>(
      "POST",
      `/v1/calls/${encodeURIComponent(callId)}/messages`,
      body,
    );
  }

  /**
   * The call's stored transcript, oldest first, until its content is
   * deleted. With `q`, the matching entries, best first.
   */
  history(callId: string, query: CallHistoryQuery = {}): Promise<CallEntry[]> {
    const qs = new URLSearchParams();
    if (query.after !== undefined) qs.set("after", String(query.after));
    if (query.limit !== undefined) qs.set("limit", String(query.limit));
    if (query.q) qs.set("q", query.q);
    const q = qs.toString();
    return this.chakra.relayRequest<CallEntry[]>(
      "GET",
      `/v1/calls/${encodeURIComponent(callId)}/messages${q ? `?${q}` : ""}`,
    );
  }

  /** The call's current memory bundle (call-memory mode). */
  memory(callId: string): Promise<CallMemory> {
    return this.chakra.relayRequest<CallMemory>(
      "GET",
      `/v1/calls/${encodeURIComponent(callId)}/memory`,
    );
  }

  /**
   * Ask the call's memory a question: returns the messages most likely
   * to answer it, verbatim, to answer from yourself. 503
   * `chk.calls.memory_unavailable` when the call's memory has no recall.
   */
  ask(callId: string, body: AskCallMemoryRequest): Promise<CallRecall> {
    return this.chakra.relayRequest<CallRecall>(
      "POST",
      `/v1/calls/${encodeURIComponent(callId)}/memory/ask`,
      body,
    );
  }

  /**
   * Claim the agent's undelivered call entries, oldest first (at most
   * 25); each is delivered once.
   */
  async pull(
    agentId: string,
    opts: { limit?: number; wait?: number; signal?: AbortSignal } = {},
  ): Promise<CallFeedItem[]> {
    return (await this.pullWithMeta(agentId, opts)).items;
  }

  /**
   * `pull`, plus `waited`: the seconds the relay held the request open
   * (`ChakraMCP-Wait`), 0 when it didn't wait, `null` from servers before
   * 0.7.
   */
  async pullWithMeta(
    agentId: string,
    opts: { limit?: number; wait?: number; signal?: AbortSignal } = {},
  ): Promise<PullResult<CallFeedItem>> {
    const { body, headers } = await this.chakra.relayGetWithHeaders<CallFeedItem[]>(
      `/v1/inbox/calls?${pullQuery(agentId, { limit: opts.limit, wait: opts.wait })}`,
      opts.signal,
    );
    return { items: body, waited: waitedFrom(headers) };
  }

  /**
   * Long-running loop for a pull agent: pull the calls feed and handle
   * each item in order until `signal` fires.
   *
   * - `invite`: `onInvite` accepts or declines (default: accept).
   * - `message`: `onMessage`'s reply, if any, is sent as the agent.
   * - `accepted`, `declined`, `ended`, `missed`: `onEvent`, if given.
   *
   * An answer or reply to a call that ended meanwhile is dropped. A
   * handler that throws, or another failed answer or reply, goes to
   * `onError` and the loop continues. A pulled batch is already claimed,
   * so it finishes before the loop stops.
   */
  async serve(agentId: string, opts: CallServeOptions): Promise<void> {
    const interval = opts.pollIntervalMs ?? 2000;
    const limit = Math.min(opts.batchSize ?? 25, 25);
    const wait = opts.waitSecs ?? 25;
    const report = opts.onError ?? defaultOnCallError;
    while (!opts.signal?.aborted) {
      let pulled: PullResult<CallFeedItem>;
      try {
        pulled = await this.pullWithMeta(agentId, { limit, wait, signal: opts.signal });
      } catch (err) {
        if (opts.signal?.aborted) break;
        report(err);
        await sleep(interval, opts.signal);
        continue;
      }
      const batch = pulled.items;
      if (batch.length === 0) {
        if (!relayWaited(pulled)) await sleep(interval, opts.signal);
        continue;
      }
      // In order: a call's entries build on the ones before them.
      for (const item of batch) {
        try {
          await this.dispatch(agentId, item, opts);
        } catch (err) {
          report(err, item);
        }
      }
    }
  }

  private async dispatch(agentId: string, item: CallFeedItem, opts: CallServeOptions): Promise<void> {
    const callId = item.call.id;
    switch (item.entry.kind) {
      case "invite": {
        const decision = opts.onInvite ? await opts.onInvite(item) : undefined;
        try {
          if (decision?.accept === false) {
            await this.decline(callId, { agent_id: agentId, reason: decision.reason });
          } else {
            await this.accept(callId, { agent_id: agentId, message: decision?.message });
          }
        } catch (err) {
          // The caller hung up, or it was missed, meanwhile.
          if (isNotActive(err)) return;
          throw err;
        }
        return;
      }
      case "message": {
        const reply = await opts.onMessage(item);
        if (reply === undefined || reply === null) return;
        const body: SendCallMessageRequest =
          typeof reply === "string"
            ? { agent_id: agentId, text: reply }
            : { agent_id: agentId, text: reply.text, data: reply.data };
        try {
          await this.send(callId, body);
        } catch (err) {
          // The call ended meanwhile: there's no one to reply to.
          if (isNotActive(err)) return;
          throw err;
        }
        return;
      }
      case "accepted":
      case "declined":
      case "ended":
      case "missed":
        await opts.onEvent?.(item);
        return;
      default:
        // `invocation` entries aren't delivered; skip kinds newer than this SDK.
        return;
    }
  }
}

// Re-export the AgentSummary too for downstream consumers building DTOs.
export type { AgentSummary };
