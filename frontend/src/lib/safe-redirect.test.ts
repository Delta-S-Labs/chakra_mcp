import { expect, test } from "@playwright/test";
import { DEFAULT_REDIRECT, safeRedirect } from "./safe-redirect";

test.describe("safeRedirect keeps paths on this site", () => {
  for (const path of [
    "/app",
    "/app/agents?tab=mine#top",
    "/invites/abc123",
    "/app/pair?session=ABCD-1234",
    // The consent screen's own bounce: an encoded https:// inside the query.
    "/oauth/authorize?response_type=code&client_id=mcp_x&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&state=s%2Bt",
  ]) {
    test(path, () => {
      expect(safeRedirect(path)).toBe(path);
    });
  }

  test("normalizes dot segments instead of passing them through", () => {
    expect(safeRedirect("/app/../oauth/authorize")).toBe("/oauth/authorize");
  });
});

test.describe("safeRedirect refuses anything that could leave the site", () => {
  for (const target of [
    "https://example.com/app",
    "http://example.com",
    "HTTPS://example.com",
    "//example.com",
    "//example.com/app",
    "/\\example.com",
    "\\\\example.com",
    "/\t/example.com",
    "/\n/example.com",
    "/%2Fexample.com",
    "/%2f%2fexample.com",
    "/%5Cexample.com",
    "/%09/example.com",
    "/%E0%A4%A",
    "javascript:alert(1)",
    "JavaScript:alert(1)",
    " javascript:alert(1)",
    "java\tscript:alert(1)",
    "data:text/html,hi",
    "app",
    "",
  ]) {
    test(JSON.stringify(target), () => {
      expect(safeRedirect(target)).toBe(DEFAULT_REDIRECT);
    });
  }

  test("refuses a repeated query parameter and other non-strings", () => {
    expect(safeRedirect(["/app", "https://example.com"])).toBe(DEFAULT_REDIRECT);
    expect(safeRedirect(undefined)).toBe(DEFAULT_REDIRECT);
    expect(safeRedirect(null)).toBe(DEFAULT_REDIRECT);
  });

  test("uses the given fallback", () => {
    expect(safeRedirect("https://example.com", "/app/pair")).toBe("/app/pair");
  });
});
