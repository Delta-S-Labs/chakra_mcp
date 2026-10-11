"use client";

import { useEffect, useRef, useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { startCheckout } from "@/app/(app)/app/credits/actions";
import type { PaymentStatus, PurchaseInfo } from "@/lib/api";
import { pollVerdict, returnHint, type ReturnHint } from "@/lib/checkout-return";
import { callsFor, creditsMcFor, formatUsd, parseDollars } from "@/lib/credits-math";
import { formatCredits } from "@/lib/format";
import styles from "./credits.module.css";

/** What the box is doing. Only the backend says whether credits landed:
 *  Dodo's overlay events and return-URL parameters are hints at most. */
type Phase =
  | { kind: "idle" }
  | { kind: "starting" }
  | { kind: "paying" }
  | { kind: "confirming"; checkoutId: string; hint: ReturnHint }
  | { kind: "done"; creditsMc: number }
  | { kind: "pending" }
  | { kind: "declined" }
  | { kind: "review" }
  | { kind: "error"; message: string };

const POLL_EVERY_MS = 2_000;

type Dodo = typeof import("dodopayments-checkout").DodoPayments;

/**
 * Buy credits for one account: any amount within the server's limits, paid
 * in Dodo's checkout overlay. When the payment completes or fails, Dodo
 * sends the browser back here with `?checkout=<id>&status=…`, and the box
 * polls until the backend knows the outcome (Dodo's signed webhook credits
 * it, not this page).
 */
export function BuyCredits({
  slug,
  purchase,
  costPerInvocationMc,
  returnedCheckoutId,
  returnedStatus,
}: {
  slug: string;
  purchase: PurchaseInfo;
  costPerInvocationMc: number;
  /** Set when Dodo sent the buyer back from a checkout. */
  returnedCheckoutId?: string;
  /** Dodo's `status` on the return URL, a hint only. */
  returnedStatus?: string;
}) {
  const router = useRouter();
  const [, startTransition] = useTransition();
  const [amount, setAmount] = useState("10");
  const [phase, setPhase] = useState<Phase>(
    returnedCheckoutId
      ? { kind: "confirming", checkoutId: returnedCheckoutId, hint: returnHint(returnedStatus) }
      : { kind: "idle" },
  );
  // The overlay's event callback is registered once, so it reads the
  // current phase through a ref.
  const phaseRef = useRef(phase);
  useEffect(() => {
    phaseRef.current = phase;
  }, [phase]);
  const dodo = useRef<Dodo | null>(null);

  const cents = parseDollars(amount);
  const inRange = cents !== null && cents >= purchase.min_cents && cents <= purchase.max_cents;
  const boughtMc = cents !== null ? creditsMcFor(cents, purchase.credits_per_usd) : 0;
  const busy = phase.kind === "starting" || phase.kind === "paying" || phase.kind === "confirming";

  // After a return from Dodo: poll until the backend knows the outcome
  // (pollVerdict decides when to stop).
  useEffect(() => {
    if (phase.kind !== "confirming") return;
    const { checkoutId, hint } = phase;
    const started = Date.now();
    let last: PaymentStatus | null = null;
    let creditsMc = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;

    const settle = (next: Phase) => {
      setPhase(next);
      // Fresh balance, payments and history, and drop `?checkout=` so a
      // reload doesn't start confirming again.
      startTransition(() => router.replace(`/app/credits?account=${encodeURIComponent(slug)}`));
    };

    const tick = async () => {
      try {
        const res = await fetch(
          `/api/credits/checkout-status?account=${encodeURIComponent(slug)}&id=${encodeURIComponent(checkoutId)}`,
          { cache: "no-store" },
        );
        if (res.ok) {
          const body = (await res.json()) as { status: PaymentStatus; credits_mc: number };
          last = body.status;
          creditsMc = body.credits_mc;
        } else if (res.status === 404) {
          return settle({ kind: "error", message: "That checkout isn't one of this account's." });
        }
      } catch {
        // A network blip: keep polling.
      }
      if (stopped) return;
      switch (pollVerdict(hint, last, Date.now() - started)) {
        case "done":
          return settle({ kind: "done", creditsMc });
        case "review":
          return settle({ kind: "review" });
        case "declined":
          return settle({ kind: "declined" });
        case "pending":
          return settle({ kind: "pending" });
        case "keep":
          timer = setTimeout(tick, POLL_EVERY_MS);
      }
    };

    timer = setTimeout(tick, 0);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [phase, router, slug]);

  async function overlay(): Promise<Dodo> {
    if (dodo.current) return dodo.current;
    const { DodoPayments } = await import("dodopayments-checkout");
    DodoPayments.Initialize({
      mode: purchase.mode,
      displayType: "overlay",
      onEvent: (event) => {
        if (event.event_type === "checkout.closed" && phaseRef.current.kind === "paying") {
          setPhase({ kind: "idle" });
        } else if (event.event_type === "checkout.error") {
          const message = typeof event.data?.message === "string" ? event.data.message : null;
          setPhase({ kind: "error", message: message ?? "The checkout ran into a problem. Try again." });
        }
      },
    });
    dodo.current = DodoPayments;
    return DodoPayments;
  }

  async function buy() {
    if (!inRange || cents === null || busy) return;
    setPhase({ kind: "starting" });
    const result = await startCheckout(slug, cents);
    if (!result.ok) {
      setPhase({ kind: "error", message: result.error });
      return;
    }
    if (!isDodoCheckout(result.checkoutUrl)) {
      setPhase({ kind: "error", message: "The checkout address looked wrong, so it wasn't opened." });
      return;
    }
    try {
      const sdk = await overlay();
      setPhase({ kind: "paying" });
      sdk.Checkout.open({ checkoutUrl: result.checkoutUrl });
    } catch {
      setPhase({ kind: "error", message: "Couldn't open the checkout. Try again." });
    }
  }

  return (
    <section className={styles.buyCard} aria-labelledby="buy-credits-title">
      <div className={styles.buyHead}>
        <h3 id="buy-credits-title" className={styles.label}>
          Buy credits
        </h3>
        {purchase.mode === "test" && <span className={styles.pillButter}>test mode</span>}
      </div>

      <form
        className={styles.buyForm}
        onSubmit={(e) => {
          e.preventDefault();
          void buy();
        }}
      >
        <label className={styles.amountField}>
          <span className={styles.amountDollar} aria-hidden="true">
            $
          </span>
          <input
            className={styles.amountInput}
            inputMode="decimal"
            autoComplete="off"
            aria-label="Amount in US dollars"
            aria-invalid={!inRange}
            aria-describedby="buy-credits-preview"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            disabled={busy}
          />
        </label>
        <button type="submit" className={styles.buyButton} disabled={!inRange || busy}>
          {phase.kind === "starting" ? "Opening checkout…" : "Buy"}
        </button>
      </form>

      <p id="buy-credits-preview" className={styles.buyPreview}>
        {inRange && cents !== null
          ? `${formatUsd(cents)} → ${formatCredits(boughtMc)} credits (${callsFor(boughtMc, costPerInvocationMc).toLocaleString("en-US")} calls)`
          : `Any amount from ${formatUsd(purchase.min_cents)} to ${formatUsd(purchase.max_cents)}.`}
      </p>

      <BuyMessage phase={phase} />

      <p className={styles.buyFoot}>
        Paid through Dodo Payments, which handles tax and emails your receipt. Credits never
        expire.
      </p>
    </section>
  );
}

function BuyMessage({ phase }: { phase: Phase }) {
  switch (phase.kind) {
    case "confirming":
      return (
        <p className={styles.buyStatus} role="status">
          {phase.hint === "succeeded" ? "Payment received, adding credits…" : "Checking the payment…"}
        </p>
      );
    case "done":
      return (
        <p className={styles.buyOk} role="status">
          Added {formatCredits(phase.creditsMc)} credits.
        </p>
      );
    case "pending":
      return (
        <p className={styles.buyStatus} role="status">
          We&apos;ll add the credits as soon as Dodo confirms the payment. Check Payments below.
        </p>
      );
    case "declined":
      return (
        <p className={styles.warn} role="alert">
          The payment didn&apos;t go through. You can try again; if Dodo did charge you, the
          credits will still arrive.
        </p>
      );
    case "review":
      return (
        <p className={styles.warn} role="alert">
          The payment needs a manual check. We&apos;ll sort it out.
        </p>
      );
    case "error":
      return (
        <p className={styles.warn} role="alert">
          {phase.message}
        </p>
      );
    default:
      return null;
  }
}

/** Only ever open Dodo's own checkout pages. */
function isDodoCheckout(url: string): boolean {
  try {
    const { protocol, hostname } = new URL(url);
    return protocol === "https:" && (hostname === "dodopayments.com" || hostname.endsWith(".dodopayments.com"));
  } catch {
    return false;
  }
}
