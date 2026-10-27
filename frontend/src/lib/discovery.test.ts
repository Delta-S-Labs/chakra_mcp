import { expect, test } from "@playwright/test";
import { formatAgo, parseTags } from "./discovery";

test("last seen reads coarsely", () => {
  const now = Date.parse("2026-10-27T12:00:00Z");
  expect(formatAgo("2026-10-27T11:59:30Z", now)).toBe("just now");
  expect(formatAgo("2026-10-27T11:55:00Z", now)).toBe("5m ago");
  expect(formatAgo("2026-10-27T09:00:00Z", now)).toBe("3h ago");
  expect(formatAgo("2026-10-25T12:00:00Z", now)).toBe("2d ago");
  expect(formatAgo("2026-06-27T12:00:00Z", now)).toBe("4mo ago");
  expect(formatAgo("not a date", now)).toBe("just now");
});

test("tags are split, lowercased and checked as the relay checks them", () => {
  expect(parseTags("travel, Booking  b2b,travel")).toEqual({ tags: ["travel", "booking", "b2b"] });
  expect(parseTags("")).toEqual({ tags: [] });
  expect(parseTags("under_score").problem).toContain("under_score");
  expect(parseTags("x".repeat(33)).problem).toContain("1-32");
  expect(parseTags(Array.from({ length: 11 }, (_, i) => `t${i}`).join(" ")).problem).toBe(
    "At most 10 tags.",
  );
});
