"use client";

import type { ConsentSettingsInput, MaxValidity } from "@/lib/relay";
import styles from "../agents.module.css";

export interface Member {
  user_id: string;
  display_name: string;
  role: "owner" | "admin" | "member";
}

export interface ConsentDraft {
  required: boolean;
  /** `human_in_loop`: a person confirms each reply. */
  humanReply: boolean;
  approvers: string[];
  maxValidity: MaxValidity;
  wait: string;
}

export const DEFAULT_CONSENT: ConsentDraft = {
  required: false,
  humanReply: false,
  approvers: [],
  maxValidity: "once",
  wait: "1d",
};

const MAX_VALIDITY: { value: MaxValidity; label: string }[] = [
  { value: "once", label: "Each request on its own" },
  { value: "1h", label: "Up to 1 hour" },
  { value: "1d", label: "Up to 1 day" },
  { value: "7d", label: "Up to 7 days" },
  { value: "30d", label: "Up to 30 days" },
];

const WAIT: { value: string; label: string }[] = [
  { value: "1h", label: "1 hour" },
  { value: "1d", label: "1 day" },
  { value: "3d", label: "3 days" },
  { value: "7d", label: "7 days" },
];

/** The draft's `semantics`. */
export function semanticsOf(d: ConsentDraft): "autonomous" | "human_in_loop" {
  return d.humanReply ? "human_in_loop" : "autonomous";
}

/** The draft as the relay's `consent` input. */
export function consentInput(d: ConsentDraft): ConsentSettingsInput {
  return {
    required: d.required,
    approvers: d.approvers,
    max_validity: d.maxValidity,
    wait: d.wait,
  };
}

/**
 * The Approvals settings (owner consent): "Ask before delivering requests"
 * (requests wait for an approver's decision on the web; the agent gets a
 * notice with the link) and "A person writes the reply" (an agent's reply
 * is a draft until an approver confirms it). Approvers and how long things
 * wait apply to both. Push-mode agents can't: the relay forwards their
 * calls at once.
 */
export function ConsentFields({
  value,
  onChange,
  members,
  pushMode,
}: {
  value: ConsentDraft;
  onChange: (next: ConsentDraft) => void;
  members: Member[];
  pushMode: boolean;
}) {
  if (pushMode) {
    return (
      <p className={styles.formHint}>
        Asking before delivering requests needs a pull-mode agent. This one takes calls at its own
        endpoint, so the relay can&apos;t hold them for approval.
      </p>
    );
  }
  const waitOptions = WAIT.some((w) => w.value === value.wait)
    ? WAIT
    : [...WAIT, { value: value.wait, label: value.wait }];
  return (
    <>
      <label className={styles.toggle}>
        <input
          type="checkbox"
          checked={value.required}
          onChange={(e) => onChange({ ...value, required: e.target.checked })}
        />
        <span>
          Ask before delivering requests
          <span className={styles.toggleHint}>
            Requests wait until you approve them, one at a time or for a while. The agent gets a
            link to send you.
          </span>
        </span>
      </label>
      <label className={styles.toggle}>
        <input
          type="checkbox"
          checked={value.humanReply}
          onChange={(e) => onChange({ ...value, humanReply: e.target.checked })}
        />
        <span>
          A person writes the reply
          <span className={styles.toggleHint}>
            Your agent&apos;s replies wait until you confirm them, so callers know a person
            stood behind each answer.
          </span>
        </span>
      </label>
      {value.required && (
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Longest approval</span>
          <select
            value={value.maxValidity}
            onChange={(e) => onChange({ ...value, maxValidity: e.target.value as MaxValidity })}
          >
            {MAX_VALIDITY.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
      )}
      {(value.required || value.humanReply) && (
        <>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Wait for a decision</span>
            <select value={value.wait} onChange={(e) => onChange({ ...value, wait: e.target.value })}>
              {waitOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          {members.length > 1 && (
            <fieldset className={styles.approverList}>
              <legend className={styles.fieldLabel}>
                Who approves{" "}
                <span className={styles.formHint}>
                  {value.approvers.length === 0 ? "(owners and admins)" : ""}
                </span>
              </legend>
              {members.map((m) => (
                <label key={m.user_id} className={styles.approverRow}>
                  <input
                    type="checkbox"
                    checked={value.approvers.includes(m.user_id)}
                    onChange={(e) =>
                      onChange({
                        ...value,
                        approvers: e.target.checked
                          ? [...value.approvers, m.user_id]
                          : value.approvers.filter((id) => id !== m.user_id),
                      })
                    }
                  />
                  <span>
                    {m.display_name} <span className={styles.formHint}>{m.role}</span>
                  </span>
                </label>
              ))}
            </fieldset>
          )}
        </>
      )}
    </>
  );
}
