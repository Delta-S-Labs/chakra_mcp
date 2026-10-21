// chakramcp.com's house agents: Ping and the ChakraMCP Guide.
//
// Runs as one process beside the relay. Claims invocations from both agents'
// inboxes, answers them, and befriends anyone who asks. See README.md.

import Anthropic from "@anthropic-ai/sdk";
import { ChakraMCP } from "@chakramcp/sdk";
import { ensureAgents, type HouseAgent } from "./agents.js";
import { ConfigError, loadConfig } from "./config.js";
import { openDb } from "./db.js";
import { syncFriends } from "./friends.js";
import { makeClaude } from "./guide.js";
import { handleGuide, handlePing, MAX_PENDING_TOTAL, type Blocklist } from "./handlers.js";
import { KeyedLock, Semaphore } from "./keyed-queue.js";
import { guideLimits, HOUR_MS, Limiter } from "./limits.js";
import { errorName, log } from "./log.js";
import { pause, pullLoop } from "./relay.js";
import { Sessions } from "./sessions.js";
import { buildSystemPrompt, loadDocs, promptVersion } from "./system-prompt.js";

const FRIENDS_EVERY_MS = 30_000;
/** Guide questions answered at once, across all callers. */
const GUIDE_CONCURRENCY = 2;

async function main(): Promise<void> {
  let config;
  try {
    config = loadConfig();
  } catch (err) {
    if (err instanceof ConfigError) {
      log.error("config.invalid", { problem: err.message });
      process.exit(1);
    }
    throw err;
  }

  const system = buildSystemPrompt(loadDocs());
  const version = promptVersion(system, config.model);
  const db = openDb(config.dataDir);
  const limiter = new Limiter(db);
  const sessions = new Sessions(db, version);
  const chakra = new ChakraMCP({
    apiKey: config.chakraApiKey,
    appUrl: config.appUrl,
    relayUrl: config.relayUrl,
  });
  const claude = makeClaude(
    new Anthropic({ apiKey: config.anthropicApiKey, maxRetries: 1 }),
    config.model,
    system,
  );
  // A bad request shape (or key) shows up here at boot, not on the first
  // question. Not fatal: the API may just be unreachable for a moment.
  claude
    .check()
    .then((tokens) => log.info("claude.ready", { model: config.model, system_tokens: tokens }))
    .catch((err: unknown) =>
      log.error("claude.check_failed", {
        error: errorName(err),
        status: err instanceof Anthropic.APIError ? (err.status ?? null) : null,
        detail: err instanceof Anthropic.APIError ? err.message : null,
      }),
    );
  const blocked: Blocklist = { agents: config.blockedAgents, accounts: config.blockedAccounts };

  const stop = new AbortController();
  for (const sig of ["SIGTERM", "SIGINT"] as const) {
    process.once(sig, () => {
      log.info("shutdown.begin", { signal: sig });
      stop.abort();
    });
  }

  // The relay may still be starting: keep trying.
  let house: [HouseAgent, HouseAgent] | undefined;
  for (let delay = 2000; !house && !stop.signal.aborted; delay = Math.min(delay * 2, 60_000)) {
    try {
      house = await ensureAgents(chakra, config.accountSlug);
    } catch (err) {
      log.warn("startup.ensure_agents_failed", { error: errorName(err), detail: (err as Error).message });
      await pause(delay, stop.signal);
    }
  }
  if (!house) return;
  const [ping, guide] = house;
  log.info("started", {
    ping_agent_id: ping.id,
    guide_agent_id: guide.id,
    model: config.model,
    prompt_version: version,
    prompt_chars: system.length,
    daily_budget_usd: config.dailyBudgetUsd,
  });

  const lock = new KeyedLock();
  const inFlight = new Set<Promise<void>>();
  const track = (p: Promise<void>) => {
    inFlight.add(p);
    void p.finally(() => inFlight.delete(p));
  };
  const guideDeps = {
    chakra,
    house: guide,
    limiter,
    limits: guideLimits(config.dailyBudgetUsd),
    sessions,
    claude,
    blocked,
    lock,
    slots: new Semaphore(GUIDE_CONCURRENCY),
    stopping: () => stop.signal.aborted,
  };
  const pingDeps = { chakra, house: ping, limiter, blocked };

  const loops = [
    pullLoop({
      chakra,
      agentId: ping.id,
      room: () => 10,
      onInvocation: (inv) => track(handlePing(pingDeps, inv)),
      signal: stop.signal,
    }),
    pullLoop({
      chakra,
      agentId: guide.id,
      room: () => MAX_PENDING_TOTAL - lock.total(),
      onInvocation: (inv) => track(handleGuide(guideDeps, inv)),
      signal: stop.signal,
    }),
    (async () => {
      while (!stop.signal.aborted) {
        try {
          await syncFriends(chakra, house, blocked);
        } catch (err) {
          log.warn("friends.sync_failed", { error: errorName(err) });
        }
        await pause(FRIENDS_EVERY_MS, stop.signal);
      }
    })(),
    (async () => {
      while (!stop.signal.aborted) {
        limiter.cleanup();
        sessions.cleanup();
        log.info("guide.spend", { last_24h_usd: Number(limiter.spentToday(guide.capabilityName).toFixed(4)) });
        await pause(HOUR_MS, stop.signal);
      }
    })(),
  ];

  await Promise.all(loops);
  // Pulling has stopped. Questions still queued are turned away; the ones
  // being answered get to finish and post their results.
  log.info("shutdown.draining", { in_flight: inFlight.size });
  await Promise.allSettled([...inFlight]);
  db.close();
  log.info("shutdown.done");
}

main().catch((err: unknown) => {
  log.error("fatal", { error: errorName(err), detail: err instanceof Error ? err.message : String(err) });
  process.exit(1);
});
