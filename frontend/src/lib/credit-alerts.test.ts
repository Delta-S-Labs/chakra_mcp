import { expect, test } from "@playwright/test";
import type { CreditAlert } from "./api";
import { creditBanner } from "./credit-alerts";

function alert(over: Partial<CreditAlert>): CreditAlert {
  return {
    account_slug: "acme",
    account_name: "Acme",
    account_type: "organization",
    state: "low",
    balance_mc: 35_000,
    calls_left: 350,
    next_grant_on: "2026-11-01",
    ...over,
  };
}

test("no alerts, no banner", () => {
  expect(creditBanner([], true)).toBeNull();
});

test("a low account: how many calls are left, and it can be dismissed", () => {
  const b = creditBanner([alert({})], true)!;
  expect(b.lines).toHaveLength(1);
  expect(b.lines[0].text).toBe("Acme is low on credits: about 350 calls left.");
  expect(b.lines[0].href).toBe("/app/credits?account=acme");
  expect(b.action).toBe("Buy credits");
  expect(b.dismissible).toBe(true);
});

test("the personal account is named as such", () => {
  const b = creditBanner([alert({ account_type: "individual", account_name: "Kaustav" })], false)!;
  expect(b.lines[0].text).toMatch(/^Your personal account is low/);
  expect(b.action).toBe("View credits");
});

test("out of credits: when calls resume, and buying only where it's possible", () => {
  const blocked = alert({ state: "blocked", calls_left: 0, balance_mc: -20 });
  expect(creditBanner([blocked], true)!.lines[0].text).toBe(
    "Acme is out of credits. Calls are refused until 1 Nov, or until you buy more.",
  );
  const selfHosted = creditBanner([blocked], false)!;
  expect(selfHosted.lines[0].text).toBe("Acme is out of credits. Calls are refused until 1 Nov.");
  expect(selfHosted.dismissible).toBe(false);
});

test("out of credits comes first, three lines at most, then a count", () => {
  const alerts = [
    alert({ account_slug: "a", account_name: "A" }),
    alert({ account_slug: "b", account_name: "B" }),
    alert({ account_slug: "c", account_name: "C" }),
    alert({ account_slug: "d", account_name: "D", state: "blocked", calls_left: 0 }),
    alert({ account_slug: "e", account_name: "E" }),
  ];
  const b = creditBanner(alerts, true)!;
  expect(b.lines.map((l) => l.key)).toEqual(["d", "a", "b"]);
  expect(b.more).toBe("and 2 more");
  expect(b.dismissible).toBe(false);
});

test("a dismissal lasts only while the alerts stay the same", () => {
  const one = creditBanner([alert({})], true)!.dismissKey;
  const same = creditBanner([alert({ calls_left: 300 })], true)!.dismissKey;
  const worse = creditBanner([alert({ state: "blocked", calls_left: 0 })], true)!.dismissKey;
  expect(same).toBe(one);
  expect(worse).not.toBe(one);
});

test("one call is singular", () => {
  expect(creditBanner([alert({ calls_left: 1 })], true)!.lines[0].text).toMatch(/about 1 call left/);
});
