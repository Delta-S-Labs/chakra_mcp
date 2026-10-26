import { expect, test } from "@playwright/test";
import {
  canDelete,
  canEnd,
  endedLabel,
  eventLine,
  memoryLabel,
  mySide,
  otherParty,
  partyName,
  retentionLabel,
} from "./calls";
import type { Call, CallEntry } from "./relay";

const call = (over: Partial<Call> = {}): Call => ({
  id: "c1",
  state: "active",
  caller: { agent_id: "a", account_id: "A", display_name: "Alice's agent", account_slug: "alice" },
  callee: { agent_id: "b", account_id: "B", display_name: "Bob's agent", account_slug: "bob" },
  memory: "own",
  memory_features: { tags: false, recall: false },
  ttl_secs: 30 * 86_400,
  idle_secs: 86_400,
  topic: "Venue",
  created_at: "2026-10-26T10:00:00Z",
  answered_at: null,
  ended_at: null,
  ended_by: null,
  purge_at: null,
  purged_at: null,
  message_count: 0,
  last_message_at: null,
  ...over,
});

const entry = (over: Partial<CallEntry>): CallEntry => ({
  seq: 1,
  kind: "message",
  from_agent_id: "a",
  text: null,
  data: null,
  blocked: false,
  created_at: "2026-10-26T10:00:00Z",
  ...over,
});

test("the viewer's side and the party across", () => {
  const c = call();
  expect(mySide(c, new Set(["b"]))).toBe("callee");
  expect(otherParty(c, "callee").display_name).toBe("Alice's agent");
  expect(mySide(c, new Set(["a", "b"]))).toBe("both");
  expect(mySide(c, new Set(["z"]))).toBeNull();
  expect(partyName({ agent_id: null, account_id: "A", display_name: null, account_slug: null })).toBe(
    "A deleted agent",
  );
});

test("events read as lines", () => {
  const c = call({ state: "ended", ended_by: "owner" });
  expect(eventLine(c, entry({ kind: "invite" }))).toBe("Alice's agent called Bob's agent");
  expect(eventLine(c, entry({ kind: "declined", from_agent_id: "b", text: "Busy" }))).toBe(
    "Bob's agent declined: “Busy”",
  );
  expect(eventLine(c, entry({ kind: "ended", from_agent_id: null }))).toBe("An owner ended the call");
  expect(
    eventLine(c, entry({ kind: "invocation", invocation: { id: "i", capability: "book_venue", status: "in_progress" } })),
  ).toBe("Alice's agent asked for book_venue (in progress)");
  expect(endedLabel(c)).toBe("Ended: an owner ended it");
  expect(endedLabel(call())).toBeNull();
});

test("what happens to the content", () => {
  expect(retentionLabel(call())).toBe("Content kept 30 days after the call ends");
  expect(retentionLabel(call({ ttl_secs: 0 }))).toBe("Content deleted when the call ends");
  expect(retentionLabel(call({ ttl_secs: 7200 }))).toBe("Content kept 2 hours after the call ends");
  expect(retentionLabel(call({ purge_at: "2099-11-03T00:00:00Z" }))).toBe(
    "Content kept until 3 Nov 2099",
  );
  expect(retentionLabel(call({ purged_at: "2026-11-03T00:00:00Z" }))).toBe(
    "Content deleted on 3 Nov 2026",
  );
});

test("memory and owner actions", () => {
  expect(memoryLabel(call())).toBe("Each agent keeps its own memory");
  expect(memoryLabel(call({ memory: "call", memory_features: { tags: true, recall: true } }))).toBe(
    "The relay keeps the call's memory",
  );
  expect(canEnd(call())).toBe(true);
  expect(canEnd(call({ state: "ended" }))).toBe(false);
  expect(canDelete(call({ purged_at: "2026-11-03T00:00:00Z" }))).toBe(false);
});
