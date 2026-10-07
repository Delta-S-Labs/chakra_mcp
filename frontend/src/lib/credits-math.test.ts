import { expect, test } from "@playwright/test";
import { callsFor, creditsMcFor, formatUsd, parseDollars } from "./credits-math";

test.describe("parseDollars reads what people type", () => {
  for (const [typed, cents] of [
    ["10", 1000],
    ["10.5", 1050],
    ["10.50", 1050],
    ["2.37", 237],
    [".5", 50],
    ["$5", 500],
    ["$ 5", 500],
    ["1,000.25", 100025],
    ["  7  ", 700],
    ["0", 0],
  ] as const) {
    test(JSON.stringify(typed), () => {
      expect(parseDollars(typed)).toBe(cents);
    });
  }

  for (const typed of ["", " ", "-5", "1.234", "abc", "1e3", "5.", "$", "10 dollars", "1.2.3"]) {
    test(`refuses ${JSON.stringify(typed)}`, () => {
      expect(parseDollars(typed)).toBeNull();
    });
  }
});

test("credits and calls match the server's math", () => {
  // $1 = 1,000 credits; a cent = 10 credits = 10,000 mc.
  expect(creditsMcFor(1, 1000)).toBe(10_000);
  expect(creditsMcFor(237, 1000)).toBe(2_370_000);
  expect(creditsMcFor(500_000, 1000)).toBe(5_000_000_000);
  // 0.1 credit (100 mc) per call.
  expect(callsFor(2_370_000, 100)).toBe(23_700);
  expect(callsFor(150, 100)).toBe(1);
  expect(callsFor(-50, 100)).toBe(0);
  expect(callsFor(1_000, 0)).toBe(0);
});

test("formatUsd", () => {
  expect(formatUsd(237)).toBe("$2.37");
  expect(formatUsd(100)).toBe("$1.00");
  expect(formatUsd(500_000)).toBe("$5,000.00");
});
