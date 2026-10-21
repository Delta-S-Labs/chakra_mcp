// Settings come from the environment. Secrets are never logged: a missing
// one is reported by name only.

export interface Config {
  relayUrl: string;
  appUrl: string;
  chakraApiKey: string;
  anthropicApiKey: string;
  /** The account (org) whose agents these are. */
  accountSlug: string;
  /** All Guide questions together, per rolling 24 hours. */
  dailyBudgetUsd: number;
  blockedAgents: ReadonlySet<string>;
  blockedAccounts: ReadonlySet<string>;
  dataDir: string;
  model: string;
}

export class ConfigError extends Error {}

function required(env: NodeJS.ProcessEnv, name: string): string {
  const value = env[name]?.trim();
  if (!value) throw new ConfigError(`${name} is not set`);
  return value;
}

function idSet(raw: string | undefined): ReadonlySet<string> {
  return new Set(
    (raw ?? "")
      .split(",")
      .map((s) => s.trim().toLowerCase())
      .filter(Boolean),
  );
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const budget = Number(env.HOUSE_DAILY_BUDGET_USD?.trim() || "10");
  if (!Number.isFinite(budget) || budget <= 0) {
    throw new ConfigError("HOUSE_DAILY_BUDGET_USD must be a positive number");
  }
  const chakraApiKey = required(env, "HOUSE_CHAKRAMCP_API_KEY");
  if (!chakraApiKey.startsWith("ck_")) {
    throw new ConfigError("HOUSE_CHAKRAMCP_API_KEY must be a ck_… API key");
  }
  return {
    // Inside the Compose network the relay answers on its service name.
    relayUrl: env.HOUSE_RELAY_URL?.trim() || "http://relay:8090",
    appUrl: env.HOUSE_APP_URL?.trim() || "http://relay:8080",
    chakraApiKey,
    anthropicApiKey: required(env, "HOUSE_ANTHROPIC_API_KEY"),
    accountSlug: env.HOUSE_ACCOUNT_SLUG?.trim() || "chakramcp",
    dailyBudgetUsd: budget,
    blockedAgents: idSet(env.HOUSE_BLOCKED_AGENTS),
    blockedAccounts: idSet(env.HOUSE_BLOCKED_ACCOUNTS),
    dataDir: env.HOUSE_DATA_DIR?.trim() || "/data",
    model: env.HOUSE_MODEL?.trim() || "claude-sonnet-5-5",
  };
}
