import { expect, test } from "@playwright/test";
import {
  describeTerms,
  inputFields,
  isHeldInput,
  narrowingOptions,
  timeLeft,
  validityLabel,
  validitySeconds,
} from "./consent";

test("validities parse like the relay's", () => {
  expect(validitySeconds("once")).toBe(0);
  expect(validitySeconds(undefined)).toBe(0);
  expect(validitySeconds("7d")).toBe(7 * 86400);
  expect(validitySeconds("36h")).toBe(36 * 3600);
  expect(validitySeconds("1w")).toBeNull();
  expect(validitySeconds("0h")).toBeNull();
});

test("terms read in words", () => {
  expect(describeTerms({})).toBe("Just this request");
  expect(describeTerms({ valid_for: "7d" })).toBe("For 7 days");
  expect(describeTerms({ valid_for: "1d", max_requests: 5 })).toBe("For 1 day, up to 5 requests");
  expect(describeTerms({ valid_for: "36h", max_requests: 1 })).toBe("For 36 hours, 1 request");
  expect(validityLabel("once")).toBe("Just this request");
  expect(validityLabel("1h")).toBe("1 hour");
});

test("approvers are only offered narrower choices", () => {
  expect(narrowingOptions("30d", "7d")).toEqual(["7d", "1d", "1h", "once"]);
  expect(narrowingOptions("36h", "30d")).toEqual(["36h", "1d", "1h", "once"]);
  expect(narrowingOptions("7d", "30d")).toEqual(["7d", "1d", "1h", "once"]);
  expect(narrowingOptions("once", "30d")).toEqual(["once"]);
  expect(narrowingOptions("7d", "once")).toEqual(["once"]);
  expect(narrowingOptions(undefined, "7d")).toEqual(["once"]);
});

test("time left counts down", () => {
  const now = new Date("2026-10-23T10:00:00Z");
  expect(timeLeft("2026-10-23T10:30:00Z", now)).toBe("30 minutes left");
  expect(timeLeft("2026-10-23T13:00:00Z", now)).toBe("3 hours left");
  expect(timeLeft("2026-10-26T10:00:00Z", now)).toBe("3 days left");
  expect(timeLeft("2026-10-23T09:00:00Z", now)).toBe("Expired");
});

test("held inputs are recognised and inputs split into fields", () => {
  expect(isHeldInput({ __chakramcp_held__: true })).toBe(true);
  expect(isHeldInput({ held: true })).toBe(false);
  expect(isHeldInput(null)).toBe(false);
  expect(inputFields({ date: "2026-10-28", slots: [1, 2] })).toEqual([
    { name: "date", value: "2026-10-28" },
    { name: "slots", value: "[1,2]" },
  ]);
  expect(inputFields("hi")).toEqual([{ name: "input", value: "hi" }]);
  expect(inputFields(null)).toEqual([]);
});
