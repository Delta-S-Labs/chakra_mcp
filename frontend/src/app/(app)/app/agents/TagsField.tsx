"use client";

import { parseTags, TAG_SUGGESTIONS } from "@/lib/discovery";
import styles from "./agents.module.css";

/**
 * An agent's directory tags, typed as text ("travel, booking"), with the
 * common ones offered as one-click additions (MCP and discovery spec §5.6).
 * The form checks the text with `parseTags` before saving.
 */
export function TagsField({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const { tags } = parseTags(value);
  const offered = TAG_SUGGESTIONS.filter((t) => !tags.includes(t));
  return (
    <div className={`${styles.field} ${styles.fieldFull}`}>
      <label className={styles.field}>
        <span className={styles.fieldLabel}>Tags</span>
        <input
          type="text"
          placeholder="scheduling, travel"
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
        <span className={styles.fieldHint}>
          Up to 10, so people can filter the directory by them: lowercase letters, digits and
          hyphens.
        </span>
      </label>
      {tags.length < 10 && (
        <ul className={styles.tagSuggestions} aria-label="Common tags">
          {offered.map((t) => (
            <li key={t}>
              <button
                type="button"
                onClick={() => onChange([...tags, t].join(", "))}
                className={styles.tagSuggestion}
              >
                + {t}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
