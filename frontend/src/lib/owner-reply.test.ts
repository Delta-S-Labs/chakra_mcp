import { expect, test } from "@playwright/test";
import { replyOutput } from "./owner-reply";

const now = new Date("2026-10-30T09:00:00.123Z");

test("typed replies become the call's output by shape", () => {
  expect(replyOutput("text", " See you at 3 ", now)).toEqual({ ok: true, output: "See you at 3" });
  expect(replyOutput("replied", "See you at 3", now)).toEqual({
    ok: true,
    output: { status: "replied", reply: "See you at 3", responded_at: "2026-10-30T09:00:00Z" },
  });
  expect(replyOutput("json", '{"slots":[1]}', now)).toEqual({ ok: true, output: { slots: [1] } });
});

test("blank replies and bad JSON are refused with a sentence", () => {
  expect(replyOutput("text", "   ", now)).toEqual({ ok: false, error: "Write a reply first." });
  const bad = replyOutput("json", "{nope", now);
  expect(bad.ok).toBe(false);
  if (!bad.ok) expect(bad.error).toContain("must be JSON");
});
