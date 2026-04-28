"use client";

import { useState } from "react";
import { createInvite } from "@/lib/api";
import styles from "../orgs.module.css";

export function InviteForm({ slug, token }: { slug: string; token: string }) {
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<"owner" | "admin" | "member">("member");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<{ token: string; email: string; expires_at: string } | null>(
    null,
  );

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setCreated(null);
    setPending(true);
    try {
      const result = await createInvite(token, slug, { email: email.trim(), role });
      setCreated({ token: result.token, email: result.email, expires_at: result.expires_at });
      setEmail("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to invite.");
    } finally {
      setPending(false);
    }
  }

  const inviteUrl = created
    ? `${typeof window !== "undefined" ? window.location.origin : ""}/invites/${created.token}`
    : "";

  return (
    <section className={styles.invitePanel}>
      <h2 className={styles.sectionTitle}>Invite a teammate</h2>
      <p className={styles.formHint}>
        Email delivery is a TODO. Copy the invite link and send it directly for
        now. Links expire in 7 days.
      </p>

      <form className={styles.inviteForm} onSubmit={handleSubmit}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Email</span>
          <input
            type="email"
            required
            placeholder="teammate@example.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
        </label>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Role</span>
          <select value={role} onChange={(e) => setRole(e.target.value as typeof role)}>
            <option value="member">Member</option>
            <option value="admin">Admin</option>
            <option value="owner">Owner</option>
          </select>
        </label>
        <button type="submit" className={styles.create} disabled={pending}>
          {pending ? "Creating…" : "Generate invite"}
        </button>
      </form>

      {error && <div className={styles.errorInline}>{error}</div>}

      {created && (
        <div className={styles.invite}>
          <div className={styles.inviteHead}>
            Invite ready for <strong>{created.email}</strong> - expires{" "}
            {new Date(created.expires_at).toLocaleDateString()}.
          </div>
          <code className={styles.inviteUrl}>{inviteUrl}</code>
          <button
            type="button"
            className={styles.copyBtn}
            onClick={() => navigator.clipboard.writeText(inviteUrl)}
          >
            Copy
          </button>
        </div>
      )}
    </section>
  );
}
