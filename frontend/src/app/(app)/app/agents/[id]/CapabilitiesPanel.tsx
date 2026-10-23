"use client";

import { useState } from "react";
import {
  createCapability,
  deleteCapability,
  updateCapability,
  type Capability,
  type Visibility,
} from "@/lib/relay";
import {
  ConsentFields,
  consentInput,
  DEFAULT_CONSENT,
  semanticsOf,
  type ConsentDraft,
  type Member,
} from "./ConsentFields";
import styles from "../agents.module.css";

/** A capability's stored consent settings as an editable draft. */
function draftOf(cap: Capability): ConsentDraft {
  const c = cap.consent;
  const humanReply = cap.semantics === "human_in_loop";
  if (!c) return { ...DEFAULT_CONSENT, humanReply };
  return {
    required: c.required,
    humanReply,
    approvers: c.approvers ?? [],
    maxValidity: c.max_validity,
    wait: c.wait,
  };
}

export function CapabilitiesPanel({
  token,
  agentId,
  canEdit,
  initial,
  members = [],
  pushMode = false,
}: {
  token: string;
  agentId: string;
  canEdit: boolean;
  initial: Capability[];
  /** The agent's account members, to pick approvers from. */
  members?: Member[];
  /** Push-mode agents can't ask their owner first. */
  pushMode?: boolean;
}) {
  const [items, setItems] = useState<Capability[]>(initial);
  // Owner consent: "Ask before delivering requests" on the add form, and
  // the capability whose approval settings are open for editing.
  const [consent, setConsent] = useState<ConsentDraft>(DEFAULT_CONSENT);
  const [editing, setEditing] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState<ConsentDraft>(DEFAULT_CONSENT);
  const [saving, setSaving] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [inputSchema, setInputSchema] = useState("{}");
  const [outputSchema, setOutputSchema] = useState("{}");
  const [visibility, setVisibility] = useState<Visibility>("network");
  // Migration 0022: opt this capability into being callable by any
  // registered agent without a friendship/grant. The toggle only
  // appears when visibility is `network` (matches the DB CHECK
  // `cap_public_requires_network`).
  const [publicInvoke, setPublicInvoke] = useState(false);
  const [monthlyQuota, setMonthlyQuota] = useState<number | "">("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function parseJson(label: string, raw: string): Record<string, unknown> {
    try {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>;
      }
      throw new Error(`${label} must be a JSON object`);
    } catch {
      throw new Error(`${label} is not valid JSON`);
    }
  }

  async function handleAdd(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    let input: Record<string, unknown>;
    let output: Record<string, unknown>;
    try {
      input = parseJson("Input schema", inputSchema);
      output = parseJson("Output schema", outputSchema);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Schema parse failed.");
      return;
    }
    // Pre-validate public-invoke combo so the user sees a clear error
    // rather than waiting for the relay's 400. The relay validates
    // identically; this is just UX.
    if (publicInvoke) {
      if (visibility !== "network") {
        setError("Public-invoke requires visibility 'network'.");
        return;
      }
      if (monthlyQuota === "" || monthlyQuota < 1) {
        setError("Monthly quota must be at least 1 when public-invoke is on.");
        return;
      }
    }
    setPending(true);
    try {
      const created = await createCapability(token, agentId, {
        name: name.trim(),
        description: description.trim(),
        input_schema: input,
        output_schema: output,
        visibility,
        ...(publicInvoke
          ? {
              public_invoke: true,
              public_monthly_quota_per_agent: monthlyQuota as number,
            }
          : {}),
        ...(consent.required || consent.humanReply ? { consent: consentInput(consent) } : {}),
        semantics: semanticsOf(consent),
      });
      setItems((prev) => [...prev, created].sort((a, b) => a.name.localeCompare(b.name)));
      setName("");
      setDescription("");
      setInputSchema("{}");
      setOutputSchema("{}");
      setVisibility("network");
      setPublicInvoke(false);
      setMonthlyQuota("");
      setConsent(DEFAULT_CONSENT);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't add capability.");
    } finally {
      setPending(false);
    }
  }

  async function handleSaveConsent(cap: Capability) {
    setError(null);
    setSaving(true);
    try {
      const updated = await updateCapability(token, agentId, cap.id, {
        consent: consentInput(editDraft),
        semantics: semanticsOf(editDraft),
      });
      setItems((prev) => prev.map((c) => (c.id === cap.id ? updated : c)));
      setEditing(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't save the approval settings.");
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete(cap: Capability) {
    if (!confirm(`Remove capability '${cap.name}'?`)) return;
    setError(null);
    try {
      await deleteCapability(token, agentId, cap.id);
      setItems((prev) => prev.filter((c) => c.id !== cap.id));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't remove.");
    }
  }

  return (
    <section className={styles.capPanel}>
      <h2 className={styles.sectionTitle}>
        Capabilities <span className={styles.count}>{items.length}</span>
      </h2>

      {items.length === 0 ? (
        <div className={styles.emptyCard}>
          <p className={styles.emptyHead}>
            No capabilities yet.
          </p>
          <p className={styles.emptyBody}>
            Capabilities are the typed operations friends can invoke against
            this agent (<code>schedule_meeting</code>, <code>summarize</code>,
            etc., each with input + output JSON Schemas). Until at least one is
            published, peers have nothing to call — friendships and grants
            still work, but no traffic can flow, and your usage charts on{" "}
            <a className={styles.emptyLink} href="/app/usage">/app/usage</a>{" "}
            stay at zero.
          </p>
          {canEdit ? (
            <p className={styles.emptyBody}>
              Add one with the form below, or from a terminal:
              {" "}
              <code className={styles.emptyCmd}>chakramcp capabilities create --agent &lt;id&gt; ...</code>
              {" "}
              (<code>chakramcp cap add --help</code> for the full flag list).
            </p>
          ) : (
            <p className={styles.emptyBody}>
              Check back once the agent&apos;s owner publishes one.
            </p>
          )}
        </div>
      ) : (
        <ul className={styles.capList}>
          {items.map((cap) => (
            <li key={cap.id} className={styles.capRow}>
              <div>
                <div className={styles.rowName}>
                  <code>{cap.name}</code>{" "}
                  <span
                    className={`${styles.badge} ${
                      cap.visibility === "network" ? styles.badgeOn : ""
                    }`}
                  >
                    {cap.visibility}
                  </span>
                  {cap.public_invoke && (
                    <span
                      className={`${styles.badge} ${styles.badgeOn}`}
                      title={
                        cap.public_monthly_quota_per_agent
                          ? `Publicly invokable — ${cap.public_monthly_quota_per_agent}/month per agent.`
                          : "Publicly invokable."
                      }
                    >
                      public · {cap.public_monthly_quota_per_agent}/mo
                    </span>
                  )}
                  {cap.consent?.required && (
                    <span
                      className={`${styles.badge} ${styles.badgeOn}`}
                      title="Requests wait for the owner's approval before reaching the agent."
                    >
                      asks first
                    </span>
                  )}
                  {cap.semantics === "human_in_loop" && (
                    <span
                      className={`${styles.badge} ${styles.badgeOn}`}
                      title="A person confirms each reply before the caller sees it."
                    >
                      person replies
                    </span>
                  )}
                </div>
                {cap.description && (
                  <div className={styles.rowMeta}>{cap.description}</div>
                )}
                {editing === cap.id && (
                  <div className={styles.consentEditor}>
                    <ConsentFields
                      value={editDraft}
                      onChange={setEditDraft}
                      members={members}
                      pushMode={pushMode}
                    />
                    {!pushMode && (
                      <div className={styles.editorActions}>
                        <button
                          type="button"
                          className={styles.create}
                          disabled={saving}
                          onClick={() => handleSaveConsent(cap)}
                        >
                          {saving ? "Saving…" : "Save"}
                        </button>
                        <button
                          type="button"
                          className={styles.dangerLink}
                          onClick={() => setEditing(null)}
                        >
                          Cancel
                        </button>
                      </div>
                    )}
                  </div>
                )}
              </div>
              {canEdit && (
                <div className={styles.editorActions} style={{ gridColumn: "auto" }}>
                  {editing !== cap.id && (
                    <button
                      type="button"
                      className={styles.dangerLink}
                      onClick={() => {
                        setEditDraft(draftOf(cap));
                        setEditing(cap.id);
                      }}
                    >
                      Approvals
                    </button>
                  )}
                  <button
                    type="button"
                    className={styles.dangerLink}
                    onClick={() => handleDelete(cap)}
                  >
                    Remove
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}

      {canEdit && (
        <form className={styles.capForm} onSubmit={handleAdd}>
          <header className={styles.formHead}>
            <h3 className={styles.subTitle}>Add a capability</h3>
            <p className={styles.formHint}>
              Names are snake_case. Schemas are JSON Schema objects (use{" "}
              <code>{"{}"}</code> for &quot;no constraint&quot;).
            </p>
          </header>

          <div className={styles.capFields}>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Name</span>
              <input
                type="text"
                required
                placeholder="schedule_meeting"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>

            <label className={styles.field}>
              <span className={styles.fieldLabel}>Visibility</span>
              <select
                value={visibility}
                onChange={(e) => {
                  const v = e.target.value as Visibility;
                  setVisibility(v);
                  // Flipping away from "network" auto-clears the
                  // public-invoke toggle so the form can't submit
                  // a state the server's CHECKs would reject.
                  if (v !== "network") {
                    setPublicInvoke(false);
                    setMonthlyQuota("");
                  }
                }}
              >
                <option value="network">Network</option>
                <option value="private">Private</option>
              </select>
            </label>

            {/* Migration 0022: public-invoke toggle. Only rendered for
                network-visible capabilities so we never show users a
                control whose state the relay would 400 on. */}
            {visibility === "network" && (
              <label className={styles.toggle}>
                <input
                  type="checkbox"
                  checked={publicInvoke}
                  onChange={(e) => {
                    setPublicInvoke(e.target.checked);
                    if (!e.target.checked) setMonthlyQuota("");
                  }}
                />
                <span>Publicly invokable — any agent can call it without a friendship</span>
              </label>
            )}
            {visibility === "network" && publicInvoke && (
              <label className={styles.field}>
                <span className={styles.fieldLabel}>Monthly quota per agent</span>
                <input
                  type="number"
                  min={1}
                  required
                  value={monthlyQuota}
                  onChange={(e) =>
                    setMonthlyQuota(e.target.value === "" ? "" : Number(e.target.value))
                  }
                  placeholder="100"
                />
              </label>
            )}

            <ConsentFields
              value={consent}
              onChange={setConsent}
              members={members}
              pushMode={pushMode}
            />

            <label className={`${styles.field} ${styles.fieldFull}`}>
              <span className={styles.fieldLabel}>Description</span>
              <input
                type="text"
                placeholder="One-line summary"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
              />
            </label>

            <label className={`${styles.field} ${styles.fieldFull}`}>
              <span className={styles.fieldLabel}>Input schema (JSON)</span>
              <textarea
                rows={3}
                value={inputSchema}
                onChange={(e) => setInputSchema(e.target.value)}
              />
            </label>

            <label className={`${styles.field} ${styles.fieldFull}`}>
              <span className={styles.fieldLabel}>Output schema (JSON)</span>
              <textarea
                rows={3}
                value={outputSchema}
                onChange={(e) => setOutputSchema(e.target.value)}
              />
            </label>

            <button type="submit" className={styles.create} disabled={pending}>
              {pending ? "Adding…" : "Add capability"}
            </button>
          </div>

          {error && <div className={styles.errorInline}>{error}</div>}
        </form>
      )}
    </section>
  );
}
