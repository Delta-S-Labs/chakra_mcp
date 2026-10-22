import assert from "node:assert/strict";
import { test } from "node:test";
import { openDb } from "./db.js";
import { DAY_MS, guideLimits, HOUR_MS, Limiter } from "./limits.js";
import { clock } from "./test-helpers.js";

test("an agent gets 10 Guide questions an hour, then a retry time", () => {
  const c = clock();
  const limiter = new Limiter(openDb(null), c.now);
  const limits = guideLimits(10);
  for (let i = 0; i < 10; i++) {
    assert.deepEqual(limiter.check("guide.ask", "a", "acct", limits), { ok: true });
    limiter.record("guide.ask", "a", "acct");
    c.advance(60_000);
  }
  const v = limiter.check("guide.ask", "a", "acct", limits);
  assert.equal(v.ok, false);
  // The first call was 10 minutes ago: it rolls off in 50 minutes.
  assert.equal(!v.ok && v.retryAfterSecs, 50 * 60);
  assert.equal(!v.ok && v.limit, "agent_hour");
  // Another agent is unaffected.
  assert.deepEqual(limiter.check("guide.ask", "b", "acct2", limits), { ok: true });
  c.advance(50 * 60_000);
  assert.deepEqual(limiter.check("guide.ask", "a", "acct", limits), { ok: true });
});

test("spend limits: per agent, per account and for everyone", () => {
  const c = clock();
  const limiter = new Limiter(openDb(null), c.now);
  const limits = guideLimits(3);
  const id = limiter.record("guide.ask", "a", "acct");
  limiter.addCost(id, 1.0);
  let v = limiter.check("guide.ask", "a", "acct", limits);
  assert.ok(!v.ok && v.reason.includes("this agent") && v.limit === "agent_usd");
  assert.ok(!v.ok && v.retryAfterSecs === DAY_MS / 1000);

  // A second agent in the same account: the account's $2 is reached next.
  limiter.addCost(limiter.record("guide.ask", "b", "acct"), 1.0);
  v = limiter.check("guide.ask", "c", "acct", limits);
  assert.ok(!v.ok && v.reason.includes("account") && v.limit === "account_usd");

  // Everyone's $3 budget.
  limiter.addCost(limiter.record("guide.ask", "d", "other"), 1.0);
  v = limiter.check("guide.ask", "e", "third", limits);
  assert.ok(!v.ok && v.reason.includes("budget") && v.limit === "global_usd");
  assert.equal(limiter.spentToday("guide.ask"), 3);
});

test("limits are per capability, and cleanup forgets old calls", () => {
  const c = clock();
  const db = openDb(null);
  const limiter = new Limiter(db, c.now);
  for (let i = 0; i < 100; i++) limiter.record("ping.echo", "a", null);
  assert.equal(limiter.check("ping.echo", "a", null, { perAgentHour: 100 }).ok, false);
  assert.equal(limiter.check("guide.ask", "a", null, { perAgentHour: 100 }).ok, true);
  c.advance(HOUR_MS + 1);
  assert.equal(limiter.check("ping.echo", "a", null, { perAgentHour: 100 }).ok, true);
  c.advance(2 * DAY_MS);
  limiter.cleanup();
  assert.equal((db.prepare("SELECT COUNT(*) AS n FROM calls").get() as { n: number }).n, 0);
});
