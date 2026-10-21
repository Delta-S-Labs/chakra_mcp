import { DatabaseSync } from "node:sqlite";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

// One SQLite file holds the Guide's conversations and every call's count and
// cost, so limits and sessions survive a restart.
export function openDb(dataDir: string | null): DatabaseSync {
  let path = ":memory:";
  if (dataDir) {
    mkdirSync(dataDir, { recursive: true });
    path = join(dataDir, "house-agents.sqlite");
  }
  const db = new DatabaseSync(path);
  db.exec(`
    PRAGMA journal_mode = WAL;
    PRAGMA busy_timeout = 5000;
    CREATE TABLE IF NOT EXISTS sessions (
      agent_id TEXT PRIMARY KEY,
      prompt_version TEXT NOT NULL,
      messages TEXT NOT NULL,
      bytes INTEGER NOT NULL,
      turns INTEGER NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS calls (
      id INTEGER PRIMARY KEY,
      capability TEXT NOT NULL,
      agent_id TEXT NOT NULL,
      account_id TEXT,
      at INTEGER NOT NULL,
      cost_usd REAL NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS calls_by_agent ON calls (capability, agent_id, at);
    CREATE INDEX IF NOT EXISTS calls_by_account ON calls (capability, account_id, at);
    CREATE INDEX IF NOT EXISTS calls_by_time ON calls (capability, at);
  `);
  return db;
}
