import { expect, test } from "@playwright/test";
import { CONFIRM_WAIT_MS, NOT_PAID_WAIT_MS, pollVerdict, returnHint } from "./checkout-return";

test("returnHint reads Dodo's status", () => {
  expect(returnHint("succeeded")).toBe("succeeded");
  expect(returnHint("failed")).toBe("not_paid");
  expect(returnHint("cancelled")).toBe("not_paid");
  expect(returnHint("expired")).toBe("not_paid");
  expect(returnHint("processing")).toBe("unknown");
  expect(returnHint("requires_customer_action")).toBe("unknown");
  expect(returnHint(undefined)).toBe("unknown");
  expect(returnHint(["succeeded"])).toBe("unknown");
});

test.describe("pollVerdict", () => {
  test("the backend decides, whatever the URL says", () => {
    for (const hint of ["succeeded", "not_paid", "unknown"] as const) {
      expect(pollVerdict(hint, "paid", 0)).toBe("done");
      expect(pollVerdict(hint, "unapplied", 0)).toBe("review");
    }
  });

  test("a declined payment is reported as soon as the backend has it", () => {
    expect(pollVerdict("not_paid", null, 0)).toBe("keep");
    expect(pollVerdict("not_paid", "open", 2_000)).toBe("keep");
    expect(pollVerdict("not_paid", "failed", 2_000)).toBe("declined");
    expect(pollVerdict("not_paid", "expired", 2_000)).toBe("declined");
  });

  test("a declined payment is reported after a short wait even without the webhook", () => {
    expect(pollVerdict("not_paid", "open", NOT_PAID_WAIT_MS - 1)).toBe("keep");
    expect(pollVerdict("not_paid", "open", NOT_PAID_WAIT_MS)).toBe("declined");
    expect(pollVerdict("not_paid", null, NOT_PAID_WAIT_MS)).toBe("declined");
  });

  test("after a success, an earlier decline doesn't end the wait", () => {
    expect(pollVerdict("succeeded", "failed", 30_000)).toBe("keep");
    expect(pollVerdict("succeeded", "open", 30_000)).toBe("keep");
    expect(pollVerdict("unknown", "failed", 30_000)).toBe("keep");
  });

  test("the long wait ends as pending, or declined if the backend says so", () => {
    expect(pollVerdict("succeeded", "open", CONFIRM_WAIT_MS)).toBe("pending");
    expect(pollVerdict("succeeded", null, CONFIRM_WAIT_MS)).toBe("pending");
    expect(pollVerdict("unknown", "failed", CONFIRM_WAIT_MS)).toBe("declined");
  });
});
