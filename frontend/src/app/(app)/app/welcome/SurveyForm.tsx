"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { submitSurvey, type SubmitSurveyRequest } from "@/lib/api";
import styles from "./welcome.module.css";

const USE_CASES: Array<{ id: string; label: string; hint: string }> = [
  { id: "personal-tools", label: "Personal tools", hint: "Agents that work for me - calendar, mail, research." },
  { id: "internal-tooling", label: "Internal tooling", hint: "Agents that help my team or company." },
  { id: "customer-product", label: "Customer-facing product", hint: "Agents shipped to end users." },
  { id: "research", label: "Research", hint: "Exploring agent capabilities, papers, prototypes." },
];

const AGENT_TYPES: Array<{ id: string; label: string }> = [
  { id: "coding", label: "Coding" },
  { id: "research", label: "Research" },
  { id: "calendar", label: "Calendar / scheduling" },
  { id: "support", label: "Support / customer ops" },
  { id: "voice", label: "Voice" },
  { id: "browser", label: "Browser-use" },
  { id: "robotics", label: "Robotics / IoT" },
  { id: "other", label: "Other" },
];

const FRAMEWORKS: Array<{ id: string; label: string }> = [
  { id: "langchain", label: "LangChain" },
  { id: "llamaindex", label: "LlamaIndex" },
  { id: "mastra", label: "Mastra" },
  { id: "vercel-ai-sdk", label: "Vercel AI SDK" },
  { id: "rig", label: "Rig" },
  { id: "crewai", label: "CrewAI" },
  { id: "autogen", label: "AutoGen" },
  { id: "custom", label: "Custom / from scratch" },
];

const SCALES: Array<{ id: SubmitSurveyRequest["scale"]; label: string }> = [
  { id: "exploring", label: "Just exploring" },
  { id: "team", label: "A team is using it" },
  { id: "company", label: "Company-wide" },
  { id: "production", label: "In production with users" },
];

export function SurveyForm({ token }: { token: string }) {
  const router = useRouter();
  const [useCase, setUseCase] = useState<string | null>(null);
  const [agentTypes, setAgentTypes] = useState<Set<string>>(new Set());
  const [frameworks, setFrameworks] = useState<Set<string>>(new Set());
  const [scale, setScale] = useState<SubmitSurveyRequest["scale"]>(null);
  const [notes, setNotes] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function toggle(set: Set<string>, value: string, setter: (s: Set<string>) => void) {
    const next = new Set(set);
    if (next.has(value)) next.delete(value);
    else next.add(value);
    setter(next);
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setPending(true);
    try {
      await submitSurvey(token, {
        use_case: useCase,
        agent_types: Array.from(agentTypes),
        frameworks: Array.from(frameworks),
        scale,
        notes: notes.trim() || null,
      });
      router.push("/app");
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't save your answers.");
      setPending(false);
    }
  }

  return (
    <form className={styles.form} onSubmit={handleSubmit}>
      <fieldset className={styles.fieldset}>
        <legend className={styles.legend}>What kind of work?</legend>
        <div className={styles.options}>
          {USE_CASES.map((u) => (
            <label key={u.id} className={styles.option}>
              <input
                type="radio"
                name="use_case"
                value={u.id}
                checked={useCase === u.id}
                onChange={() => setUseCase(u.id)}
              />
              <span className={styles.optionLabel}>{u.label}</span>
              <span className={styles.optionHint}>{u.hint}</span>
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className={styles.fieldset}>
        <legend className={styles.legend}>What kind of agents? <span className={styles.legendHint}>Pick any that fit.</span></legend>
        <div className={styles.chips}>
          {AGENT_TYPES.map((t) => (
            <label
              key={t.id}
              className={`${styles.chip} ${agentTypes.has(t.id) ? styles.chipOn : ""}`}
            >
              <input
                type="checkbox"
                checked={agentTypes.has(t.id)}
                onChange={() => toggle(agentTypes, t.id, setAgentTypes)}
              />
              {t.label}
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className={styles.fieldset}>
        <legend className={styles.legend}>Which frameworks? <span className={styles.legendHint}>Pick any that fit.</span></legend>
        <div className={styles.chips}>
          {FRAMEWORKS.map((f) => (
            <label
              key={f.id}
              className={`${styles.chip} ${frameworks.has(f.id) ? styles.chipOn : ""}`}
            >
              <input
                type="checkbox"
                checked={frameworks.has(f.id)}
                onChange={() => toggle(frameworks, f.id, setFrameworks)}
              />
              {f.label}
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className={styles.fieldset}>
        <legend className={styles.legend}>Where are you with it?</legend>
        <div className={styles.options}>
          {SCALES.map((s) => (
            <label key={s.id ?? "none"} className={styles.option}>
              <input
                type="radio"
                name="scale"
                value={s.id ?? ""}
                checked={scale === s.id}
                onChange={() => setScale(s.id)}
              />
              <span className={styles.optionLabel}>{s.label}</span>
            </label>
          ))}
        </div>
      </fieldset>

      <label className={styles.field}>
        <span className={styles.fieldLabel}>Anything else? <span className={styles.legendHint}>Optional.</span></span>
        <textarea
          rows={3}
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          placeholder="What's the one thing you'd want this network to do for you?"
        />
      </label>

      {error && <div className={styles.error}>{error}</div>}

      <div className={styles.actions}>
        <button type="submit" className={styles.submit} disabled={pending}>
          {pending ? "Saving…" : "Done - take me in"}
        </button>
      </div>
    </form>
  );
}
