"use client";

import { useState } from "react";
import {
  createCapability,
  deleteCapability,
  type Capability,
  type Visibility,
} from "@/lib/relay";
import styles from "../agents.module.css";

export function CapabilitiesPanel({
  token,
  agentId,
  canEdit,
  initial,
}: {
  token: string;
  agentId: string;
  canEdit: boolean;
  initial: Capability[];
}) {
  const [items, setItems] = useState<Capability[]>(initial);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [inputSchema, setInputSchema] = useState("{}");
  const [outputSchema, setOutputSchema] = useState("{}");
  const [visibility, setVisibility] = useState<Visibility>("network");
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
    setPending(true);
    try {
      const created = await createCapability(token, agentId, {
        name: name.trim(),
        description: description.trim(),
        input_schema: input,
        output_schema: output,
        visibility,
      });
      setItems((prev) => [...prev, created].sort((a, b) => a.name.localeCompare(b.name)));
      setName("");
      setDescription("");
      setInputSchema("{}");
      setOutputSchema("{}");
      setVisibility("network");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't add capability.");
    } finally {
      setPending(false);
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
        <p className={styles.empty}>
          No capabilities yet. {canEdit ? "Add the first one below." : "Check back soon."}
        </p>
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
                </div>
                {cap.description && (
                  <div className={styles.rowMeta}>{cap.description}</div>
                )}
              </div>
              {canEdit && (
                <button
                  type="button"
                  className={styles.dangerLink}
                  onClick={() => handleDelete(cap)}
                >
                  Remove
                </button>
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
                onChange={(e) => setVisibility(e.target.value as Visibility)}
              >
                <option value="network">Network</option>
                <option value="private">Private</option>
              </select>
            </label>

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
