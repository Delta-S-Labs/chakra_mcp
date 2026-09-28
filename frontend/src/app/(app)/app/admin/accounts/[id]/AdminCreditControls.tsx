"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import type { CreditSettingsRequest, CreditsView } from "@/lib/api";
import { creditsToMc } from "@/lib/format";
import { addCreditEntry, updateCreditSettings, type ActionResult } from "./actions";
import styles from "./controls.module.css";

/**
 * The operator's credit controls for one account: add credits, adjust
 * the balance either way, and override the monthly grant, rate limit or
 * the unlimited switch. Every change lands in the account's history.
 */
export function AdminCreditControls({
  accountId,
  view,
}: {
  accountId: string;
  view: CreditsView;
}) {
  return (
    <div className={styles.grid}>
      <EntryForm accountId={accountId} kind="grant" />
      <EntryForm accountId={accountId} kind="adjustment" />
      {/* Remount on change so the fields show the saved settings. */}
      <SettingsForm
        key={`${view.monthly_free_grant_override_mc}:${view.rate_limit_override_per_min}:${view.unlimited}`}
        accountId={accountId}
        view={view}
      />
    </div>
  );
}

function EntryForm({ accountId, kind }: { accountId: string; kind: "grant" | "adjustment" }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [amount, setAmount] = useState("");
  const [note, setNote] = useState("");
  const [result, setResult] = useState<ActionResult | null>(null);
  const isGrant = kind === "grant";

  function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    const credits = Number(amount);
    if (!Number.isFinite(credits) || creditsToMc(credits) === 0 || (isGrant && credits < 0)) {
      setResult({
        ok: false,
        error: isGrant
          ? "Enter a positive number of credits."
          : "Enter a non-zero number of credits (negative removes them).",
      });
      return;
    }
    if (!isGrant && !note.trim()) {
      setResult({ ok: false, error: "Adjustments need a note saying why." });
      return;
    }
    setResult(null);
    startTransition(async () => {
      const r = await addCreditEntry(accountId, {
        kind,
        amount_mc: creditsToMc(credits),
        note: note.trim() || null,
      });
      setResult(r);
      if (r.ok) {
        setAmount("");
        setNote("");
        router.refresh();
      }
    });
  }

  return (
    <form className={styles.card} onSubmit={onSubmit}>
      <h3 className={styles.cardTitle}>{isGrant ? "Add credits" : "Adjust balance"}</h3>
      <p className={styles.hint}>
        {isGrant
          ? "Adds to the balance. An account that was out of credits works again within seconds."
          : "Moves the balance either way, e.g. to correct after a refund. Negative removes credits."}
      </p>
      <label className={styles.field}>
        <span className={styles.fieldLabel}>Credits</span>
        <input
          type="number"
          inputMode="decimal"
          step="0.001"
          min={isGrant ? "0.001" : undefined}
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          placeholder={isGrant ? "100" : "-25"}
          disabled={pending}
          required
        />
      </label>
      <label className={styles.field}>
        <span className={styles.fieldLabel}>Note{isGrant ? " (optional)" : ""}</span>
        <input
          type="text"
          maxLength={500}
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder={isGrant ? "Welcome bonus" : "Refund for order 42"}
          disabled={pending}
        />
      </label>
      <p className={styles.fine}>Members of the account see the note.</p>
      {result && !result.ok && <div className={styles.errorLine}>{result.error}</div>}
      {result?.ok && <div className={styles.successLine}>Saved.</div>}
      <button type="submit" className={styles.submit} disabled={pending || !amount}>
        {pending ? "Saving…" : isGrant ? "Add credits" : "Adjust"}
      </button>
    </form>
  );
}

function SettingsForm({ accountId, view }: { accountId: string; view: CreditsView }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [grant, setGrant] = useState(
    view.monthly_free_grant_override_mc === null
      ? ""
      : String(view.monthly_free_grant_override_mc / 1000),
  );
  const [rate, setRate] = useState(
    view.rate_limit_override_per_min === null ? "" : String(view.rate_limit_override_per_min),
  );
  const [unlimited, setUnlimited] = useState(view.unlimited);
  const [note, setNote] = useState("");
  const [result, setResult] = useState<ActionResult | null>(null);

  function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    const body: CreditSettingsRequest = {};

    const grantMc = grant.trim() === "" ? null : creditsToMc(Number(grant));
    if (grantMc !== null && !(Number.isFinite(grantMc) && grantMc >= 0)) {
      setResult({ ok: false, error: "The monthly grant must be 0 or more credits." });
      return;
    }
    if (grantMc !== view.monthly_free_grant_override_mc) body.monthly_free_grant_mc = grantMc;

    const ratePerMin = rate.trim() === "" ? null : Number(rate);
    if (ratePerMin !== null && !(Number.isInteger(ratePerMin) && ratePerMin >= 1)) {
      setResult({ ok: false, error: "The rate limit must be a whole number, at least 1." });
      return;
    }
    if (ratePerMin !== view.rate_limit_override_per_min) body.rate_limit_per_min = ratePerMin;

    if (unlimited !== view.unlimited) body.unlimited = unlimited;

    if (Object.keys(body).length === 0) {
      setResult({ ok: false, error: "Nothing changed." });
      return;
    }
    body.note = note.trim() || null;
    setResult(null);
    startTransition(async () => {
      const r = await updateCreditSettings(accountId, body);
      setResult(r);
      if (r.ok) router.refresh();
    });
  }

  return (
    <form className={`${styles.card} ${styles.wide}`} onSubmit={onSubmit}>
      <h3 className={styles.cardTitle}>Settings</h3>
      <p className={styles.hint}>
        Per-account overrides. Leave a field blank to use the global default.
      </p>
      <div className={styles.row}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Monthly free grant (credits)</span>
          <input
            type="number"
            inputMode="decimal"
            step="0.001"
            min="0"
            value={grant}
            onChange={(e) => setGrant(e.target.value)}
            placeholder="Default"
            disabled={pending}
          />
        </label>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Rate limit (per minute)</span>
          <input
            type="number"
            inputMode="numeric"
            step="1"
            min="1"
            value={rate}
            onChange={(e) => setRate(e.target.value)}
            placeholder="Default"
            disabled={pending}
          />
        </label>
      </div>
      <label className={styles.check}>
        <input
          type="checkbox"
          checked={unlimited}
          onChange={(e) => setUnlimited(e.target.checked)}
          disabled={pending}
        />
        <span>
          Unlimited: never block this account for credits (it&apos;s still
          charged and granted, and rate limits still apply).
        </span>
      </label>
      <label className={styles.field}>
        <span className={styles.fieldLabel}>Note (optional)</span>
        <input
          type="text"
          maxLength={500}
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="Design partner"
          disabled={pending}
        />
      </label>
      {result && !result.ok && <div className={styles.errorLine}>{result.error}</div>}
      {result?.ok && <div className={styles.successLine}>Saved.</div>}
      <button type="submit" className={styles.submit} disabled={pending}>
        {pending ? "Saving…" : "Save settings"}
      </button>
    </form>
  );
}
