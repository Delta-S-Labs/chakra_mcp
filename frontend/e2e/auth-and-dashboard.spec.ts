/**
 * End-to-end smoke test for the ChakraMCP auth + agent + dashboard flow.
 *
 * Runs against a manually-started local stack:
 *   - Postgres on :5432 (task db:up)
 *   - chakramcp-server on :8080 + :8090
 *   - Frontend dev server on :3000
 *
 * The spec is ONE ordered sequence, organised into `test.describe`
 * blocks per phase. Workers = 1 + fullyParallel = false (see
 * playwright.config.ts) enforce the ordering.
 *
 * Screenshot evidence lands under `e2e/screenshots/<run-id>/phase-*.png`.
 * `test.afterAll` runs idempotent cleanup so a repeat run starts clean.
 *
 * Auth path is email + password — no OAuth providers involved. The
 * frontend's CAPTCHA_ENABLED must be `false` (set in
 * `frontend/.env.local`) or the password form gates on a captcha widget
 * we can't drive. README documents the prerequisite.
 */

import { test, expect, type Page, type Cookie } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

// Serial mode: every test runs on the same worker (so module-scope
// `state` is shared) AND a failure short-circuits the rest of the
// spec instead of marching on with `if (!state.foo) throw`. We still
// keep the top-level `afterAll` for cleanup; in serial mode it runs
// once at the end of the file regardless of which test failed.
test.describe.configure({ mode: "serial" });

import {
  signupWithPassword,
  deviceAuthorize,
  pollDeviceToken,
  revokeApiKey,
  revokePairing,
  listMyAgents,
  createAgent,
  deleteAgent,
  createCapability,
  proposeFriendship,
  acceptFriendship,
  createGrant,
  invoke,
  pullInbox,
  reportResult,
  getMe,
  APP_BASE_URL,
} from "./fixtures/api";
import { waitForStack } from "./fixtures/stack";

// ─── Test-run state ──────────────────────────────────────
// Per-spec mutable bag. We keep IDs/tokens here so the `afterAll`
// cleanup can tear down whatever phases got far enough to create
// something. Every cleanup step is wrapped in try/catch so partial
// state from a crash doesn't block the rest.
interface State {
  email: string;
  password: string;
  name: string;
  /** JWT issued to the human at /v1/auth/signup. Authoritative user creds. */
  userToken?: string;
  userId?: string;
  /** Personal account where everything lives. */
  accountId?: string;
  accountSlug?: string;
  /** The device-flow agent created during pair consent (Phase 2). */
  pairedAgentId?: string;
  pairedAgentSlug?: string;
  pairedAgentJwt?: string;
  /** The peer agent created in Phase 4 to grant a capability. */
  peerAgentId?: string;
  peerAgentSlug?: string;
  peerCapabilityId?: string;
  peerCapabilityName?: string;
  grantId?: string;
  friendshipId?: string;
  /** The API key created via the UI in Phase 3. */
  apiKeyId?: string;
  apiKeyPlaintext?: string;
  /** Device-flow pairing row (for revoke in cleanup). */
  pairingId?: string;
}

const TEST_RUN_TIMESTAMP = Date.now();
const state: State = {
  email: `smoke-test-${TEST_RUN_TIMESTAMP}@chakramcp.local`,
  password: "smoke-test-p@ssw0rd-1234",
  name: `Smoke Test ${TEST_RUN_TIMESTAMP}`,
};

const SCREENSHOTS_DIR =
  process.env.PLAYWRIGHT_SCREENSHOTS_DIR ??
  path.join(__dirname, "screenshots", "ad-hoc");

// Make sure the dir exists before any phase tries to write a milestone PNG.
fs.mkdirSync(SCREENSHOTS_DIR, { recursive: true });

// Each `test()` gets a fresh browser context. We persist the NextAuth
// cookie produced in Phase 1 here and reload it in every subsequent
// phase via `await context.addCookies(...)` (helper below).
const STORAGE_STATE_FILE = path.join(SCREENSHOTS_DIR, "auth-storage-state.json");

async function restoreAuthCookies(page: Page): Promise<void> {
  if (!fs.existsSync(STORAGE_STATE_FILE)) {
    throw new Error(
      `auth storage state file missing at ${STORAGE_STATE_FILE} — did Phase 1 run?`,
    );
  }
  const parsed = JSON.parse(fs.readFileSync(STORAGE_STATE_FILE, "utf-8")) as {
    cookies: Cookie[];
  };
  await page.context().addCookies(parsed.cookies);
}

async function shot(page: Page, name: string): Promise<string> {
  const file = path.join(SCREENSHOTS_DIR, `${name}.png`);
  await page.screenshot({ path: file, fullPage: true });
  console.log(`[e2e] screenshot → ${file}`);
  return file;
}

/**
 * Drive the /login UI to install a NextAuth session cookie in the
 * Playwright browser. Pre-condition: the user already exists (signup
 * happens out-of-band via /v1/auth/signup).
 */
async function signInThroughUi(page: Page): Promise<void> {
  await page.goto("/login");
  // The form is hidden behind "Sign in with email + password". Click to
  // reveal it. If the captcha widget were enabled this would still be
  // disabled — we error loudly in that case.
  await page.getByRole("button", { name: /sign in with email/i }).click();
  await page.getByLabel(/email/i).fill(state.email);
  await page.getByLabel(/password/i).fill(state.password);
  // The Sign in button stays disabled while captchaReady === false.
  // If CAPTCHA_ENABLED isn't `false` in the frontend env, this will time
  // out with an obvious "still disabled" failure.
  const submit = page.getByRole("button", { name: /^sign in$/i });
  await expect(submit).toBeEnabled({ timeout: 5_000 });
  await Promise.all([
    page.waitForURL(/\/app(\/|$)/, { timeout: 20_000 }),
    submit.click(),
  ]);
}

// ─── Phase 0 — sanity check the local stack is up ────────

test.beforeAll(async () => {
  await waitForStack();
});

// ─── Phase 1: Email signup + signed-in dashboard ─────────

test.describe("Phase 1 — email signup + dashboard", () => {
  test("signup populates the dashboard with the user's name", async ({ page }) => {
    // Create the user against the backend directly. This bypasses the
    // captcha widget on the /signup form and gives us the JWT we'll
    // use for every backend mutation in later phases.
    const auth = await signupWithPassword({
      email: state.email,
      password: state.password,
      name: state.name,
    });
    state.userToken = auth.token;
    state.userId = auth.user.id;
    expect(auth.memberships.length).toBeGreaterThan(0);
    state.accountId = auth.memberships[0]!.account_id;
    state.accountSlug = auth.memberships[0]!.slug;

    // Same email maps to the same /v1/me row.
    const me = await getMe(auth.token);
    expect(me.user.email).toBe(state.email);

    // Now drive the UI sign-in so the browser gets a NextAuth cookie.
    await signInThroughUi(page);
    await expect(page).toHaveURL(/\/app(\/|$)/);

    // The dashboard greets the user by first name, and the stat-card
    // grid is always rendered (even at all-zero counts).
    const firstName = state.name.split(" ")[0]!;
    await expect(page.locator("body")).toContainText(firstName);
    // The "Agents" label appears twice (sidebar nav + stat card); the
    // stat-card grid renders `Active grants`, `Pending friendships`,
    // and `In-flight inbox` labels that are unique on the page.
    await expect(page.getByText("Active grants", { exact: true })).toBeVisible();
    await expect(page.getByText("Pending friendships", { exact: true })).toBeVisible();
    await expect(page.getByText("In-flight inbox", { exact: true })).toBeVisible();

    await shot(page, "phase1-dashboard");

    // Persist the NextAuth session cookie for the remaining phases.
    // Subsequent tests get a fresh browser context per Playwright's
    // default, so we restore the cookies via `restoreAuthCookies` at
    // the top of each phase that touches the UI.
    const storage = await page.context().storageState();
    fs.writeFileSync(STORAGE_STATE_FILE, JSON.stringify(storage, null, 2));
  });
});

// ─── Phase 2: Device-flow pair ───────────────────────────

test.describe("Phase 2 — device-flow pair", () => {
  test("device authorization + approve via UI mints a JWT", async ({ page, request }) => {
    if (!state.userToken) throw new Error("Phase 1 must have run first");

    // Start the device flow. `verification_uri_complete` is the URL
    // the agent would print/QR for the human to open.
    const auth = await deviceAuthorize({
      persona: "e2e-test",
      agent_slug_hint: `e2e-agent-${TEST_RUN_TIMESTAMP}`,
      agent_display_name_hint: "E2E Test Agent",
      agent_description_hint: "Created by the Playwright smoke test.",
    });
    console.log(
      "[e2e] device_authorization response:",
      JSON.stringify(
        {
          user_code: auth.user_code,
          verification_uri: auth.verification_uri,
          verification_uri_complete: auth.verification_uri_complete,
          verification_uri_qr: auth.verification_uri_qr,
          expires_in: auth.expires_in,
        },
        null,
        2,
      ),
    );
    // Programmatic "screenshot": dump the JSON to a file so it can be
    // diffed and grepped after the run.
    fs.writeFileSync(
      path.join(SCREENSHOTS_DIR, "phase2-device-auth.json"),
      JSON.stringify(auth, null, 2),
    );

    // The verification URL points back at our frontend. We're already
    // signed in (Phase 1) — restore the cookie before the goto so the
    // consent page renders the authenticated view instead of bouncing
    // to /login.
    await restoreAuthCookies(page);
    await page.goto(auth.verification_uri_complete);
    await expect(page.getByRole("heading", { name: /approve this agent/i })).toBeVisible({
      timeout: 10_000,
    });
    await shot(page, "phase2-consent");

    // Kick the token poller and the approve click off concurrently —
    // /oauth/token returns `authorization_pending` until the human
    // clicks "Approve & create agent". Wrap pollDeviceToken in a
    // Promise so we can race it against the UI action.
    const tokenPromise = pollDeviceToken({
      deviceCode: auth.device_code,
      intervalMs: 1_500,
      timeoutMs: 30_000,
    });

    // The slug field is pre-filled from `agent_slug_hint`. Accept it.
    await page.getByRole("button", { name: /approve & create agent/i }).click();

    // Success card renders once /oauth/device-approve completes.
    await expect(page.getByText(/approved\./i)).toBeVisible({ timeout: 10_000 });
    await shot(page, "phase2-approved");

    const token = await tokenPromise;
    expect(token.access_token.length).toBeGreaterThan(20);
    expect(token.agent_id).not.toBeNull();
    expect(token.agent_slug).not.toBeNull();
    expect(token.account_slug).not.toBeNull();

    state.pairedAgentId = token.agent_id!;
    state.pairedAgentSlug = token.agent_slug!;
    state.pairedAgentJwt = token.access_token;

    // Walk the pairings list with the user token so we can grab the
    // device-flow pairing id for cleanup later.
    const pairings = await request
      .get(`${APP_BASE_URL}/v1/pairings`, {
        headers: { authorization: `Bearer ${state.userToken}` },
      })
      .then((r) => r.json());
    const ours = (pairings as Array<{ kind: string; id: string; agent_id: string | null }>).find(
      (p) => p.kind === "device_flow" && p.agent_id === state.pairedAgentId,
    );
    state.pairingId = ours?.id;
  });
});

// ─── Phase 3: API key flow ───────────────────────────────

test.describe("Phase 3 — API key creation", () => {
  test("create an API key via UI, then call /v1/me with it", async ({ page }) => {
    if (!state.userToken) throw new Error("Phase 1 must have run first");

    await restoreAuthCookies(page);
    await page.goto("/app/api-keys");

    const keyName = `e2e-smoke-test-key-${TEST_RUN_TIMESTAMP}`;
    await page.getByPlaceholder(/Local CLI/i).fill(keyName);
    // Leave the expiry default (90 days).
    await page.getByRole("button", { name: /^create key$/i }).click();

    // The just-created panel reveals the plaintext exactly once.
    const created = page.locator(":has-text('copy now, won')").first();
    await expect(created).toBeVisible({ timeout: 10_000 });
    // The plaintext sits in a <code> right after "copy now…". Grab the
    // first `ck_…` token in the visible DOM.
    const bodyText = await page.locator("body").innerText();
    const m = bodyText.match(/(ck_[A-Za-z0-9]+)/);
    expect(m, "ck_ plaintext should be visible on the page").not.toBeNull();
    state.apiKeyPlaintext = m![1]!;

    await shot(page, "phase3-key-reveal");

    // Look up the key id via the backend list — the UI shows the prefix
    // but not the full id in plain DOM text.
    const list = await import("./fixtures/api").then((m) =>
      m.listApiKeys(state.userToken!),
    );
    const match = list.find((k) => k.name === keyName);
    expect(match, `key '${keyName}' should appear in list`).toBeTruthy();
    state.apiKeyId = match!.id;

    // Round-trip the plaintext against /v1/me to prove the key auths.
    const me = await getMe(state.apiKeyPlaintext);
    expect(me.user.email).toBe(state.email);
  });
});

// ─── Phase 4: Populate the dashboard with real invocations ──

test.describe("Phase 4 — populate per-key usage by invoking a real capability", () => {
  test("create peer + capability, grant, then invoke 4× through the API key", async () => {
    if (!state.userToken || !state.pairedAgentId || !state.accountId) {
      throw new Error("Phase 1 + 2 must have run first");
    }

    // 1. Create a second test agent on the same personal account. We
    //    use the user token (full perms) rather than introducing
    //    another device-flow loop.
    const peer = await createAgent(state.userToken, {
      account_id: state.accountId,
      slug: `e2e-peer-${TEST_RUN_TIMESTAMP}`,
      display_name: `E2E Peer ${TEST_RUN_TIMESTAMP}`,
      description: "Peer agent created by the Playwright smoke test.",
      visibility: "private",
    });
    state.peerAgentId = peer.id;
    state.peerAgentSlug = peer.slug;

    // 2. Publish a trivial echo capability on the peer. Empty schemas
    //    are accepted; we just need the row to grant against.
    state.peerCapabilityName = "echo";
    const cap = await createCapability(state.userToken, peer.id, {
      name: state.peerCapabilityName,
      description: "Echo input back; used by the e2e smoke test.",
      input_schema: { type: "object" },
      output_schema: { type: "object" },
      visibility: "private",
    });
    state.peerCapabilityId = cap.id;

    // 3. Friendship: device-flow agent ↔ peer agent. Both belong to
    //    the same account, so both proposer and target can be acted
    //    on with the same user JWT.
    const friendship = await proposeFriendship(state.userToken, {
      proposer_agent_id: state.pairedAgentId,
      target_agent_id: peer.id,
      proposer_message: "e2e smoke test",
    });
    state.friendshipId = friendship.id;
    await acceptFriendship(state.userToken, friendship.id);

    // 4. Grant: peer (granter) → device-flow agent (grantee) for the
    //    echo capability.
    const grant = await createGrant(state.userToken, {
      granter_agent_id: peer.id,
      grantee_agent_id: state.pairedAgentId,
      capability_id: cap.id,
    });
    state.grantId = grant.id;

    // 5. Invoke 4× through the **API key** as caller credential. This
    //    is the bit that stamps `api_key_id` on each invocation row,
    //    which is what `/v1/api-keys/{id}/usage` filters by.
    if (!state.apiKeyPlaintext) {
      throw new Error("Phase 3 must have run first");
    }
    const invocationIds: string[] = [];
    for (let i = 0; i < 4; i++) {
      const res = await invoke(state.apiKeyPlaintext, {
        grant_id: grant.id,
        grantee_agent_id: state.pairedAgentId,
        input: { message: `hello-${i}` },
      });
      invocationIds.push(res.invocation_id);
    }

    // 6. Drain the peer's inbox so the rows reach a terminal status —
    //    cleaner test data, even though `api_key_id` is stamped at
    //    insert time and the chart populates either way.
    const inbox = await pullInbox(state.userToken, peer.id, 25);
    for (const row of inbox) {
      if (invocationIds.includes(row.id)) {
        await reportResult(state.userToken, row.id, {
          status: "succeeded",
          output: { ok: true },
        });
      }
    }

    // Verify the per-key usage endpoint reports our invocations.
    const usage = await fetch(
      `${APP_BASE_URL}/v1/api-keys/${encodeURIComponent(state.apiKeyId!)}/usage`,
      { headers: { authorization: `Bearer ${state.userToken}` } },
    ).then((r) => r.json() as Promise<{
      total_requests: number;
      by_capability_type: Array<{ capability_name: string; count: number }>;
      by_agent: Array<{ agent_slug: string; count: number }>;
    }>);
    expect(usage.total_requests).toBeGreaterThanOrEqual(4);
    expect(usage.by_capability_type.map((c) => c.capability_name)).toContain("echo");
    expect(usage.by_agent.map((a) => a.agent_slug)).toContain(state.pairedAgentSlug);
  });
});

// ─── Phase 5: Dashboard verification ─────────────────────

test.describe("Phase 5 — dashboard verification", () => {
  test("api-key detail page renders charts with real data", async ({ page }) => {
    if (!state.apiKeyId) throw new Error("Phase 3 must have run first");

    await restoreAuthCookies(page);
    await page.goto(`/app/api-keys/${state.apiKeyId}`);

    // Heading lives in <h1> with the key's display name.
    await expect(
      page.locator("h1", { hasText: `e2e-smoke-test-key-${TEST_RUN_TIMESTAMP}` }),
    ).toBeVisible({ timeout: 10_000 });

    // The usage section renders even at zero. Wait for the
    // "Usage" header explicitly so we don't race the lazy chart load.
    await expect(page.getByRole("heading", { name: /^usage$/i })).toBeVisible();

    // Total requests > 0 — phrased as "<N> requests · …".
    await expect(page.locator("p", { hasText: /\d+ request/i }).first()).toBeVisible();
    const usageMeta = await page
      .locator("p", { hasText: /\d+ request/i })
      .first()
      .innerText();
    const totalMatch = usageMeta.match(/^([\d,]+)\s*request/);
    expect(totalMatch, `expected '<N> request(s)' in usage meta, got: ${usageMeta}`).not.toBeNull();
    const total = Number(totalMatch![1]!.replace(/,/g, ""));
    expect(total).toBeGreaterThan(0);

    // Capability pie legend contains "echo".
    await expect(page.getByText(/by capability/i)).toBeVisible();
    await expect(page.locator("code", { hasText: /^echo$/ })).toBeVisible();

    // By-agent table has a row for the device-flow agent.
    await expect(page.getByRole("heading", { name: /by agent/i })).toBeVisible();
    await expect(
      page.locator("code", { hasText: state.pairedAgentSlug! }),
    ).toBeVisible();

    await shot(page, "phase5-dashboard");
  });
});

// ─── Phase 6: Cleanup ────────────────────────────────────
//
// Every step is wrapped in try/catch so a single failure doesn't strand
// the remaining resources. The order is reverse of creation: drop the
// dependent rows first (grant, friendship, capabilities), then the
// agents, then the credentials, then the pairing.

test.afterAll(async () => {
  console.log("[e2e] cleanup starting");

  // Need the user JWT to mutate relay state. If Phase 1 never ran the
  // signup we'll skip everything — there's nothing to clean.
  const token = state.userToken;
  if (!token) {
    console.log("[e2e] no user token, nothing to clean");
    return;
  }

  // 1. Revoke the API key. Idempotent on the backend (DELETE on
  //    already-revoked just no-ops).
  if (state.apiKeyId) {
    try {
      await revokeApiKey(token, state.apiKeyId);
      console.log("[e2e] api key revoked");
    } catch (err) {
      console.error("[e2e] cleanup: revokeApiKey failed", err);
    }
  }

  // 2. Revoke the device-flow pairing. The backend DELETE-cascades the
  //    minted JWT into `revoked_tokens` so the agent can't keep using it.
  if (state.pairingId) {
    try {
      await revokePairing(token, "device_flow", state.pairingId);
      console.log("[e2e] device-flow pairing revoked");
    } catch (err) {
      console.error("[e2e] cleanup: revokePairing failed", err);
    }
  }

  // 3. Delete the test agents. `DELETE /v1/agents/{id}` is a soft
  //    tombstone (migration 0009 + see relay handlers/agents.rs).
  //    Listing returns only live rows, so a successful tombstone hides
  //    the agent from the dashboard.
  for (const id of [state.peerAgentId, state.pairedAgentId]) {
    if (!id) continue;
    try {
      await deleteAgent(token, id);
      console.log(`[e2e] agent ${id} deleted (tombstoned)`);
    } catch (err) {
      console.error(`[e2e] cleanup: deleteAgent(${id}) failed`, err);
    }
  }

  // 4. Sanity-check: no e2e-smoke-test* agents remain visible to the
  //    user. We log the result rather than failing the suite — by this
  //    point Playwright has already reported pass/fail on the actual
  //    test cases.
  try {
    const remaining = await listMyAgents(token);
    const orphans = remaining.filter((a) => a.slug.startsWith("e2e-"));
    if (orphans.length > 0) {
      console.warn(
        `[e2e] cleanup left ${orphans.length} e2e- agents visible:`,
        orphans.map((a) => a.slug),
      );
    } else {
      console.log("[e2e] no e2e-* agents remaining — clean");
    }
  } catch (err) {
    console.error("[e2e] cleanup verification failed", err);
  }

  // 5. The user row itself stays — chakramcp-app does not currently
  //    expose `DELETE /v1/users/me`. Document the orphan in the
  //    README; clearing via psql is straightforward.
  console.log(
    `[e2e] cleanup done. user '${state.email}' (id=${state.userId ?? "?"}) is orphaned by design — clear via psql if needed.`,
  );
});
