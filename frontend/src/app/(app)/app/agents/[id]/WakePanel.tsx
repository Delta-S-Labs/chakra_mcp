"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import {
  clearAgentWake,
  setAgentWake,
  testAgentWake,
  type AgentWake,
  type WakeTest,
} from "@/lib/relay";
import styles from "../agents.module.css";

/**
 * A pull agent's wake URL: a URL this relay pings when work arrives for it,
 * at most every 30 seconds, so the agent needn't keep polling. JSON pings
 * carry signed counts; text pings are one line, for an ntfy topic or a chat
 * bot. The signing secret shows once, when the URL is set.
 */
export function WakePanel({
  token,
  agentId,
  initial,
}: {
  token: string;
  agentId: string;
  initial: AgentWake | null;
}) {
  const router = useRouter();
  const [wake, setWake] = useState<AgentWake | null>(initial);
  const [url, setUrl] = useState(initial?.url ?? "");
  const [format, setFormat] = useState<"json" | "text">(initial?.format ?? "json");
  const [secret, setSecret] = useState<string | null>(null);
  const [tested, setTested] = useState<WakeTest | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(step: () => Promise<void>) {
    setError(null);
    setTested(null);
    setPending(true);
    try {
      await step();
    } catch (err) {
      setError(err instanceof Error ? err.message : "That didn't work. Try again.");
    } finally {
      setPending(false);
    }
  }

  const save = (e: React.FormEvent) => {
    e.preventDefault();
    void run(async () => {
      const saved = await setAgentWake(token, agentId, { url: url.trim(), format });
      setSecret(saved.secret ?? null);
      setWake({ ...saved, secret: undefined });
      router.refresh();
    });
  };

  const test = () =>
    run(async () => {
      setTested(await testAgentWake(token, agentId));
    });

  const clear = () =>
    run(async () => {
      await clearAgentWake(token, agentId);
      setWake(null);
      setUrl("");
      setSecret(null);
      router.refresh();
    });

  return (
    <section className={styles.editPanel} aria-label="Wake URL">
      <header className={styles.formHead}>
        <h2 className={styles.sectionTitle}>Wake URL</h2>
        <p className={styles.formHint}>
          A URL this relay pings when work arrives for this agent, at most every 30 seconds,
          so it needn&apos;t keep polling. JSON pings carry signed counts for your own
          endpoint; text pings are one line, for an ntfy topic or a chat bot&apos;s
          sendMessage URL. Pings never include what was sent or who sent it.
        </p>
      </header>

      {wake && (
        <p className={styles.formHint}>
          {wake.enabled ? (
            <>
              On.{" "}
              {wake.last_ok_at
                ? `Last delivered ${new Date(wake.last_ok_at).toLocaleString()}.`
                : "Not pinged yet."}
              {wake.last_error && ` Last failure: ${wake.last_error}.`}
            </>
          ) : (
            <>
              Switched off after {wake.failures} failed pings
              {wake.last_error ? ` (last: ${wake.last_error})` : ""}. Save it again to turn it
              back on.
            </>
          )}
        </p>
      )}

      <form className={styles.fields} onSubmit={save}>
        <label className={`${styles.field} ${styles.fieldFull}`}>
          <span className={styles.fieldLabel}>URL</span>
          <input
            type="url"
            required
            placeholder="https://ntfy.sh/your-topic"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
          />
        </label>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Format</span>
          <select value={format} onChange={(e) => setFormat(e.target.value as "json" | "text")}>
            <option value="json">JSON (signed)</option>
            <option value="text">Text</option>
          </select>
        </label>
        <button type="submit" className={styles.create} disabled={pending}>
          {pending ? "Saving…" : wake ? "Save" : "Set"}
        </button>
        {wake && (
          <>
            <button type="button" className={styles.copyBtnGhost} onClick={test} disabled={pending}>
              Send a test ping
            </button>
            <button type="button" className={styles.dangerBtn} onClick={clear} disabled={pending}>
              Remove
            </button>
          </>
        )}
      </form>

      {secret && (
        <div className={styles.successInline}>
          Signing secret, shown only now: <code>{secret}</code>. JSON pings carry{" "}
          <code>ChakraMCP-Signature: t=…,v1=…</code>, the HMAC-SHA256 of{" "}
          <code>t.body</code> with this secret.
        </div>
      )}
      {tested &&
        (tested.ok ? (
          <div className={styles.successInline}>Delivered (HTTP {tested.status}).</div>
        ) : (
          <div className={styles.errorInline}>Not delivered: {tested.error ?? "no answer"}.</div>
        ))}
      {error && <div className={styles.errorInline}>{error}</div>}
    </section>
  );
}
