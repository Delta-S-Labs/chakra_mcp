import { expect, test } from "@playwright/test";
import { describeSafety, isBlocked, kindLabel, refusalMessage } from "./safety";

test("findings read in words, by part and action", () => {
  expect(describeSafety(undefined)).toEqual([]);
  expect(
    describeSafety({
      request: [
        { kind: "phone", action: "redacted", path: "input.note" },
        { kind: "phone", action: "redacted", path: "input.alt" },
        { kind: "email", action: "redacted", path: "input.note" },
        { kind: "opaque", action: "flagged", path: "input.blob" },
      ],
      reply: [{ kind: "harm_crime", action: "blocked", question: "harm_crime", p: 0.93 }],
    }),
  ).toEqual([
    "Removed from the request: 2 phone numbers, an email address.",
    "Flagged in the request: unreadable content.",
    "Blocked the reply: help with fraud or other crime.",
  ]);
});

test("blocked messages are recognised by their stand-in", () => {
  expect(isBlocked({ __chakramcp_blocked__: true })).toBe(true);
  expect(isBlocked({ note: "hi" })).toBe(false);
  expect(isBlocked(null)).toBe(false);
});

test("unknown kinds still read", () => {
  expect(kindLabel("something_new")).toBe("something new");
  expect(kindLabel("opaque", 3)).toBe("unreadable content");
});

test("refusals are explained, other errors are left alone", () => {
  expect(
    refusalMessage({
      code: "chk.policy.safety_blocked",
      message: "System One safety check blocked this note: harassment or threats (harm_harassment=0.95)",
    }),
  ).toBe("Safety screening refused this message: harassment or threats. Change it and try again.");
  expect(
    refusalMessage({
      code: "chk.policy.compliance_denied",
      message:
        "System One compliance check denied this request: request is outside the grant's stated purpose (off_purpose=0.93)",
    }),
  ).toBe(
    "Safety screening refused this message: request is outside the grant's stated purpose. Change it and try again.",
  );
  expect(refusalMessage({ code: "chk.safety.too_large", message: "x" })).toMatch(/too long/);
  expect(refusalMessage({ code: "forbidden", message: "nope" })).toBeNull();
  expect(refusalMessage(new Error("plain"))).toBeNull();
});
