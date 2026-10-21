import { expect, test } from "@playwright/test";
import { ApiClientError } from "./api";
import { RelayClientError } from "./relay";
import { isUnauthorized, sessionExpiredUrl } from "./session-expired";

test("a 401 from either backend means the session has ended", () => {
  expect(isUnauthorized(new ApiClientError(401, "unauthorized", "unauthorized"))).toBe(true);
  expect(isUnauthorized(new RelayClientError(401, "unauthorized", "unauthorized"))).toBe(true);
});

test("other statuses don't, even when the message says unauthorized", () => {
  expect(isUnauthorized(new ApiClientError(403, "forbidden", "forbidden"))).toBe(false);
  expect(isUnauthorized(new RelayClientError(500, "internal_error", "unauthorized upstream"))).toBe(false);
  expect(isUnauthorized(new ApiClientError(404, "not_found", "not found"))).toBe(false);
});

test("an error without a status is judged by its message", () => {
  expect(isUnauthorized(new Error("HTTP 401 from the app API"))).toBe(true);
  expect(isUnauthorized(new Error("Unauthorized"))).toBe(true);
  expect(isUnauthorized(new Error("fetch failed"))).toBe(false);
  expect(isUnauthorized("unauthorized")).toBe(false);
  expect(isUnauthorized(null)).toBe(false);
});

test("sign in, then back to the page", () => {
  expect(sessionExpiredUrl("/app/orgs")).toBe("/login?reason=session_expired&from=%2Fapp%2Forgs");
  expect(sessionExpiredUrl("/app/pair?code=ABCD-1234")).toBe(
    "/login?reason=session_expired&from=%2Fapp%2Fpair%3Fcode%3DABCD-1234",
  );
});
