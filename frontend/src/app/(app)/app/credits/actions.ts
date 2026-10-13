"use server";

import { auth } from "@/auth";
import { ApiClientError, createCheckout } from "@/lib/api";

export type StartCheckoutResult =
  | { ok: true; checkoutId: string; checkoutUrl: string }
  | { ok: false; error: string };

/**
 * Start buying credits for `slug`: the backend records the checkout and
 * opens Dodo's session, which the buy box shows as an overlay. Only a
 * person signed in to the web app can do this; the backend refuses API
 * keys and agent tokens.
 */
export async function startCheckout(slug: string, amountCents: number): Promise<StartCheckoutResult> {
  const session = await auth();
  const token = session?.backendToken;
  if (!token) return { ok: false, error: "Your session ended. Sign in again to buy credits." };
  if (!Number.isSafeInteger(amountCents) || amountCents <= 0) {
    return { ok: false, error: "Enter an amount in dollars." };
  }
  try {
    const created = await createCheckout(token, slug, amountCents);
    return { ok: true, checkoutId: created.checkout_id, checkoutUrl: created.checkout_url };
  } catch (err) {
    if (err instanceof ApiClientError) {
      switch (err.status) {
        case 400:
          return { ok: false, error: err.message };
        case 401:
          return { ok: false, error: "Your session ended. Sign in again to buy credits." };
        case 403:
          return { ok: false, error: "Buying credits needs you signed in to the web app." };
        case 404:
          return { ok: false, error: "Credits can't be bought for this account here." };
        case 429:
          return { ok: false, error: "Too many checkouts in the last hour. Try again later." };
        case 502:
          return { ok: false, error: "The payment provider didn't respond. Try again in a moment." };
      }
    }
    return { ok: false, error: "Couldn't start the checkout. Try again." };
  }
}
