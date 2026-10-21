import type { DatabaseSync } from "node:sqlite";
import type { BetaMessageParam } from "@anthropic-ai/sdk/resources/beta/messages/messages";
import { DAY_MS } from "./limits.js";

// The Guide's conversations, one per calling agent. A conversation's history
// is append-only: turns are added, never edited, so earlier thinking stays
// valid and the prompt cache keeps hitting. Server-side compaction keeps what
// Claude reads short; the stored history only grows until a new session.

export interface SessionOptions {
  /** A conversation idle this long starts over. */
  idleMs: number;
  /** A conversation stored this large starts over. */
  maxBytes: number;
}

export const DEFAULT_SESSION_OPTIONS: SessionOptions = {
  idleMs: 7 * DAY_MS,
  maxBytes: 2_000_000,
};

export interface Session {
  messages: BetaMessageParam[];
  turns: number;
  isNew: boolean;
}

interface Row {
  prompt_version: string;
  messages: string;
  bytes: number;
  turns: number;
  updated_at: number;
}

export class Sessions {
  constructor(
    private readonly db: DatabaseSync,
    /** Changes whenever the system prompt does; old sessions then start over. */
    private readonly promptVersion: string,
    private readonly opts: SessionOptions = DEFAULT_SESSION_OPTIONS,
    private readonly now: () => number = Date.now,
  ) {}

  load(agentId: string, startNew: boolean): Session {
    const fresh: Session = { messages: [], turns: 0, isNew: true };
    if (startNew) return fresh;
    const row = this.db
      .prepare("SELECT prompt_version, messages, bytes, turns, updated_at FROM sessions WHERE agent_id = ?")
      .get(agentId) as Row | undefined;
    if (
      !row ||
      row.prompt_version !== this.promptVersion ||
      this.now() - row.updated_at > this.opts.idleMs ||
      row.bytes > this.opts.maxBytes
    ) {
      return fresh;
    }
    try {
      return { messages: JSON.parse(row.messages) as BetaMessageParam[], turns: row.turns, isNew: false };
    } catch {
      return fresh;
    }
  }

  save(agentId: string, messages: BetaMessageParam[], turns: number, isNew: boolean): void {
    const json = JSON.stringify(messages);
    const bytes = Buffer.byteLength(json);
    const now = this.now();
    if (isNew) {
      this.db
        .prepare(
          `INSERT INTO sessions (agent_id, prompt_version, messages, bytes, turns, created_at, updated_at)
           VALUES (?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT (agent_id) DO UPDATE SET
             prompt_version = excluded.prompt_version, messages = excluded.messages,
             bytes = excluded.bytes, turns = excluded.turns,
             created_at = excluded.created_at, updated_at = excluded.updated_at`,
        )
        .run(agentId, this.promptVersion, json, bytes, turns, now, now);
    } else {
      this.db
        .prepare("UPDATE sessions SET messages = ?, bytes = ?, turns = ?, updated_at = ? WHERE agent_id = ?")
        .run(json, bytes, turns, now, agentId);
    }
  }

  /** Drops conversations nobody will resume. */
  cleanup(): void {
    this.db.prepare("DELETE FROM sessions WHERE updated_at < ?").run(this.now() - this.opts.idleMs);
  }
}
