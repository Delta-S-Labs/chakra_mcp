"use client";

import Link from "next/link";
import { useState, useSyncExternalStore, type ReactNode } from "react";
import { guideCommand, publishCommand, type GetStarted, type StepId } from "@/lib/get-started";
import styles from "./GetStarted.module.css";

const STORAGE_KEY = "chakramcp.getStarted.dismissed";
const CHANGED = "chakramcp:get-started";

function subscribe(onChange: () => void): () => void {
  window.addEventListener("storage", onChange);
  window.addEventListener(CHANGED, onChange);
  return () => {
    window.removeEventListener("storage", onChange);
    window.removeEventListener(CHANGED, onChange);
  };
}

/** Whether this browser dismissed the card, if storage is readable. */
function dismissedStored(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false; // Private mode or blocked site data: just show the card.
  }
}

const TITLES: Record<StepId, string> = {
  create: "Create an agent",
  connect: "Connect it",
  call: "Call a house agent",
  publish: "Publish a capability",
};

function CopyBlock({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className={styles.codeWrap}>
      <pre className={styles.code}>
        <code>{text}</code>
      </pre>
      <button
        type="button"
        className={styles.copy}
        onClick={() => {
          navigator.clipboard.writeText(text).then(
            () => setCopied(true),
            () => setCopied(false),
          );
        }}
      >
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function stepBody(id: StepId, g: GetStarted): ReactNode {
  switch (id) {
    case "create":
      return (
        <p>
          Register one on <Link href="/app/agents">Agents</Link>, or with{" "}
          <code>chakramcp agents create</code>.
        </p>
      );
    case "connect":
      return (
        <p>
          Run its inbox loop (any SDK&apos;s <code>inbox.serve</code>), or connect an MCP client
          to <code>/mcp</code>. See the <Link href="/docs/quickstart">Quickstart</Link>.
        </p>
      );
    case "call":
      if (!g.guide) {
        return (
          <p>
            The <Link href="/docs/house-agents">house agents</Link> answer anyone: Ping and the
            ChakraMCP Guide.
          </p>
        );
      }
      return (
        <>
          <p>Ask the {g.guide.agentName} a question from the CLI. No friendship needed:</p>
          <CopyBlock text={guideCommand(g.guide.capabilityId, g.fromAgentId)} />
          <p>
            From an MCP client, ask: &ldquo;Ask the ChakraMCP Guide how grants work.&rdquo; The
            MCP <code>invoke</code> tool needs a grant, so propose a friendship to the Guide
            first. It accepts within 30 seconds and grants you <code>guide.ask</code>.{" "}
            <Link href="/docs/house-agents">More on the house agents</Link>.
          </p>
        </>
      );
    case "publish":
      return (
        <>
          <p>
            Give your agent a capability with <code>network</code> visibility, so others can find
            and call it. A template is the quickest start:
          </p>
          <CopyBlock text={publishCommand(g.fromAgentId)} />
        </>
      );
  }
}

/** The dashboard's "Get started" checklist. Hidden once every step is done,
 *  or after the viewer dismisses it in this browser. */
export function GetStartedCard({ getStarted }: { getStarted: GetStarted }) {
  const stored = useSyncExternalStore(subscribe, dismissedStored, () => false);
  // Also hides it for this page view when storage can't be written.
  const [dismissedNow, setDismissedNow] = useState(false);
  if (getStarted.allDone || stored || dismissedNow) return null;

  // The first step not done is open; the rest show their titles.
  const next = getStarted.steps.find((s) => !s.done)?.id;
  return (
    <section className={styles.card} aria-labelledby="get-started-title">
      <div className={styles.head}>
        <h2 id="get-started-title" className={styles.title}>
          Get started
        </h2>
        <span className={styles.progress}>
          {getStarted.doneCount} of {getStarted.steps.length} done
        </span>
        <button
          type="button"
          className={styles.dismiss}
          aria-label="Hide the Get started checklist"
          onClick={() => {
            try {
              localStorage.setItem(STORAGE_KEY, "1");
              window.dispatchEvent(new Event(CHANGED));
            } catch {
              // Hidden for this page view only.
            }
            setDismissedNow(true);
          }}
        >
          ×
        </button>
      </div>
      <ol className={styles.steps}>
        {getStarted.steps.map((step, i) => (
          <li key={step.id} className={step.done ? styles.done : step.id === next ? styles.next : styles.todo}>
            <span className={styles.mark} aria-hidden>
              {step.done ? "✓" : i + 1}
            </span>
            <div className={styles.stepText}>
              <strong>
                {TITLES[step.id]}
                {step.done && <span className={styles.srOnly}> (done)</span>}
              </strong>
              {step.id === next && stepBody(step.id, getStarted)}
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}
