import type { DatabaseSync } from "node:sqlite";

// Rolling-window limits on how much each calling agent, each calling
// account, and everyone together can ask of a house agent. Counts and costs
// live in SQLite, so a restart doesn't reset them.

export const HOUR_MS = 3_600_000;
export const DAY_MS = 24 * HOUR_MS;

export interface Limits {
  perAgentHour: number;
  perAgentDay?: number;
  perAgentDayUsd?: number;
  perAccountDay?: number;
  perAccountDayUsd?: number;
  globalDay?: number;
  globalDayUsd?: number;
}

export function guideLimits(dailyBudgetUsd: number): Limits {
  return {
    perAgentHour: 10,
    perAgentDay: 50,
    perAgentDayUsd: 1,
    perAccountDay: 100,
    perAccountDayUsd: 2,
    globalDay: 1000,
    globalDayUsd: dailyBudgetUsd,
  };
}

export const PING_LIMITS: Limits = { perAgentHour: 100 };

/** Which limit refused a call, for logs and dashboards. */
export type LimitId =
  | "agent_hour"
  | "agent_day"
  | "agent_usd"
  | "account_day"
  | "account_usd"
  | "global_day"
  | "global_usd";

export type Verdict =
  | { ok: true }
  | { ok: false; limit: LimitId; reason: string; retryAfterSecs: number };

type Scope = "agent" | "account" | "all";

interface Rule {
  id: LimitId;
  scope: Scope;
  windowMs: number;
  max: number;
  usd: boolean;
  reason: string;
}

function rules(limits: Limits): Rule[] {
  const out: Rule[] = [];
  const add = (id: LimitId, scope: Scope, windowMs: number, max: number | undefined, usd: boolean, reason: string) => {
    if (max !== undefined) out.push({ id, scope, windowMs, max, usd, reason });
  };
  add("agent_hour", "agent", HOUR_MS, limits.perAgentHour, false, `at most ${limits.perAgentHour} calls an hour per agent`);
  add("agent_day", "agent", DAY_MS, limits.perAgentDay, false, `at most ${limits.perAgentDay} calls a day per agent`);
  add("agent_usd", "agent", DAY_MS, limits.perAgentDayUsd, true, "this agent has used its share for today");
  add("account_day", "account", DAY_MS, limits.perAccountDay, false, `at most ${limits.perAccountDay} calls a day per account`);
  add("account_usd", "account", DAY_MS, limits.perAccountDayUsd, true, "agents in this account have used their share for today");
  add("global_day", "all", DAY_MS, limits.globalDay, false, "the daily limit for everyone has been reached");
  add("global_usd", "all", DAY_MS, limits.globalDayUsd, true, "the daily budget for everyone has been used");
  return out;
}

export class Limiter {
  constructor(
    private readonly db: DatabaseSync,
    private readonly now: () => number = Date.now,
  ) {}

  /** Whether one more call fits. Doesn't record it: see `record`. */
  check(capability: string, agentId: string, accountId: string | null, limits: Limits): Verdict {
    const now = this.now();
    for (const rule of rules(limits)) {
      if (rule.scope === "account" && !accountId) continue;
      const since = now - rule.windowMs;
      const where = this.where(rule.scope);
      const params = this.params(capability, rule.scope, agentId, accountId, since);
      const row = this.db
        .prepare(`SELECT COUNT(*) AS n, COALESCE(SUM(cost_usd), 0) AS usd FROM calls WHERE ${where}`)
        .get(...params) as { n: number; usd: number };
      const used = rule.usd ? row.usd : row.n;
      if (used < rule.max) continue;
      // When enough of the window has rolled off to allow another call. For
      // counts that's the (n - max + 1)th oldest call; for spend, the oldest.
      const offset = rule.usd ? 0 : row.n - rule.max;
      const freed = this.db
        .prepare(`SELECT at FROM calls WHERE ${where} ORDER BY at ASC LIMIT 1 OFFSET ?`)
        .get(...params, offset) as { at: number } | undefined;
      const at = freed?.at ?? now;
      const retryAfterSecs = Math.max(1, Math.ceil((at + rule.windowMs - now) / 1000));
      return { ok: false, limit: rule.id, reason: rule.reason, retryAfterSecs };
    }
    return { ok: true };
  }

  /** Counts a call; returns its id for `addCost`. */
  record(capability: string, agentId: string, accountId: string | null): number {
    const res = this.db
      .prepare("INSERT INTO calls (capability, agent_id, account_id, at) VALUES (?, ?, ?, ?)")
      .run(capability, agentId, accountId, this.now());
    return Number(res.lastInsertRowid);
  }

  addCost(callId: number, usd: number): void {
    if (!(usd > 0)) return;
    this.db.prepare("UPDATE calls SET cost_usd = cost_usd + ? WHERE id = ?").run(usd, callId);
  }

  /** Spend across all calls of a capability in the last 24 hours. */
  spentToday(capability: string): number {
    const row = this.db
      .prepare("SELECT COALESCE(SUM(cost_usd), 0) AS usd FROM calls WHERE capability = ? AND at > ?")
      .get(capability, this.now() - DAY_MS) as { usd: number };
    return row.usd;
  }

  /** Forgets calls older than any window. */
  cleanup(): void {
    this.db.prepare("DELETE FROM calls WHERE at < ?").run(this.now() - 2 * DAY_MS);
  }

  private where(scope: Scope): string {
    if (scope === "agent") return "capability = ? AND agent_id = ? AND at > ?";
    if (scope === "account") return "capability = ? AND account_id = ? AND at > ?";
    return "capability = ? AND at > ?";
  }

  private params(
    capability: string,
    scope: Scope,
    agentId: string,
    accountId: string | null,
    since: number,
  ): Array<string | number> {
    if (scope === "agent") return [capability, agentId, since];
    if (scope === "account") return [capability, accountId ?? "", since];
    return [capability, since];
  }
}
