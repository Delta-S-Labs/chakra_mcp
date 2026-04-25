"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { createOrg } from "@/lib/api";
import styles from "./orgs.module.css";

export function CreateOrgForm({ token }: { token: string | null }) {
  const router = useRouter();
  const [slug, setSlug] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!token) {
      setError("No backend token in session.");
      return;
    }
    setError(null);
    setPending(true);
    try {
      const org = await createOrg(token, {
        slug: slug.trim(),
        display_name: name.trim(),
      });
      router.push(`/app/orgs/${org.slug}`);
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create org.");
    } finally {
      setPending(false);
    }
  }

  return (
    <form className={styles.createForm} onSubmit={handleSubmit}>
      <div className={styles.formHead}>
        <h2 className={styles.sectionTitle}>Create organization</h2>
        <p className={styles.formHint}>
          Slug becomes the URL handle (a-z, 0-9, hyphens). Display name is what
          teammates see.
        </p>
      </div>
      <div className={styles.fields}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Slug</span>
          <input
            type="text"
            required
            placeholder="acme-labs"
            pattern="[a-zA-Z0-9_-]+"
            value={slug}
            onChange={(e) => setSlug(e.target.value)}
          />
        </label>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Display name</span>
          <input
            type="text"
            required
            placeholder="Acme Labs"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <button type="submit" className={styles.create} disabled={pending}>
          {pending ? "Creating…" : "Create"}
        </button>
      </div>
      {error && <div className={styles.errorInline}>{error}</div>}
    </form>
  );
}
