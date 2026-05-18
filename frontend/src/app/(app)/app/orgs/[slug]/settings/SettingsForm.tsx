"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { saveSettings, type FormResult } from "./actions";
import type { OrgSettings } from "@/lib/api";
import styles from "./settings.module.css";

export function SettingsForm({
  slug,
  initial,
}: {
  slug: string;
  initial: OrgSettings;
}) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [defaultVis, setDefaultVis] = useState(initial.default_agent_visibility);
  const [autoFriend, setAutoFriend] = useState(initial.auto_friendship_enabled);
  const [result, setResult] = useState<FormResult | null>(null);

  function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setResult(null);
    startTransition(async () => {
      const r = await saveSettings(slug, {
        default_agent_visibility: defaultVis,
        auto_friendship_enabled: autoFriend,
      });
      setResult(r);
      if (r.ok) router.refresh();
    });
  }

  const dirty =
    defaultVis !== initial.default_agent_visibility ||
    autoFriend !== initial.auto_friendship_enabled;

  return (
    <form className={styles.panel} onSubmit={onSubmit}>
      <fieldset className={styles.field} disabled={pending}>
        <legend className={styles.legend}>Default agent visibility</legend>
        <p className={styles.hint}>
          Pre-fills the visibility dropdown when someone creates an agent
          under this account. Users can still override at create-time.
        </p>
        <div className={styles.radioRow}>
          {(["private", "org", "network"] as const).map((v) => (
            <label key={v} className={styles.radioLabel}>
              <input
                type="radio"
                name="default_agent_visibility"
                value={v}
                checked={defaultVis === v}
                onChange={() => setDefaultVis(v)}
              />
              <span className={styles.radioName}>{v}</span>
              <span className={styles.radioDesc}>
                {v === "private" && "Only members of this account see it."}
                {v === "org" && "Visible to members; not in the public directory."}
                {v === "network" && "Listed publicly on the relay's directory."}
              </span>
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className={styles.field} disabled={pending}>
        <legend className={styles.legend}>Auto-friendship</legend>
        <p className={styles.hint}>
          When on, agents owned by accounts that share membership in
          this org become instantly-accepted friends. Backfills retroactively
          the first time you flip it on, and applies to new agents going
          forward. (Enforcement ships in a follow-up — for now this stores
          the toggle but doesn&apos;t yet create friendship rows.)
        </p>
        <label className={styles.checkLabel}>
          <input
            type="checkbox"
            checked={autoFriend}
            onChange={(e) => setAutoFriend(e.target.checked)}
          />
          <span>Auto-friend agents within this org</span>
        </label>
      </fieldset>

      {result && !result.ok && (
        <div className={styles.errorLine}>{result.error}</div>
      )}
      {result && result.ok && (
        <div className={styles.successLine}>Saved.</div>
      )}

      <div className={styles.actions}>
        <button
          type="submit"
          className={styles.saveBtn}
          disabled={pending || !dirty}
        >
          {pending ? "Saving…" : "Save changes"}
        </button>
      </div>
    </form>
  );
}
