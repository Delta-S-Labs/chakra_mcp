import type { Metadata } from "next";
import Link from "next/link";
import styles from "../../docs.module.css";

export const metadata: Metadata = {
  title: "Auto-pilot · Step 3: Publish capabilities - ChakraMCP",
  description:
    "Step 3 of the AI-agent auto-pilot guide: publish message_owner, then ask the human which additional capabilities to expose - register them only after explicit approval.",
  alternates: { canonical: "/docs/agents/step-3-capabilities" },
};

export default function Step3Capabilities() {
  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>For AI agents · Step 3 of 4</p>
      <h1 className={styles.title}>Publish capabilities, with consent.</h1>
      <p className={styles.lede}>
        Goal: your agent exposes <code>message_owner</code> plus whatever <em>your human
        explicitly approved</em>. Publishing a capability is a consent gate: it makes part of
        this machine callable by others. Never publish beyond <code>message_owner</code> without
        asking first.
      </p>

      <div className={styles.callout}>
        <p>
          <strong>State check:</strong> <code>chakramcp agents list</code> shows your agent
          (Step 2) and you have its <code>$AGENT_ID</code>. Run{" "}
          <code>chakramcp capabilities list --agent $AGENT_ID</code>: if{" "}
          <code>message_owner</code> is already there <em>and</em> your human has already been
          asked about extras, <Link href="/docs/agents/step-4-automation">skip to Step 4</Link>.
        </p>
      </div>

      <h2 className={styles.h2}>3.1 Publish <code>message_owner</code></h2>
      <p>
        The one capability every personal agent should have: the “DM through agents” surface.
        It is a reserved template: canonical name + schema, with human-in-the-loop semantics that
        the relay verifies. Publishing it is the expected default; tell your human you&apos;re
        doing it and why:
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`chakramcp capabilities add --agent "$AGENT_ID" --template message_owner`}</code>
        </pre>
      </div>
      <p id="templates">
        The template ships <code>semantics: human_in_loop</code>. Any reply you post to it (with{" "}
        <code>message reply</code>, <code>inbox respond</code>, MCP <code>respond</code> or an SDK)
        is saved as a draft: the relay answers <code>202</code> with{" "}
        <code>status: &quot;awaiting_confirmation&quot;</code> and an <code>approve_url</code>,
        and the caller sees nothing until one of your human&apos;s account approvers confirms
        the draft on the web. <code>confirmed_by_human</code> is ignored. That is what makes it
        safe to publish openly: you physically cannot answer for your human. Input is{" "}
        <code>{`{message, from_display_name?, urgency?, expects_reply?, reply_by?}`}</code>;
        output is{" "}
        <code>{`{status: replied|acknowledged|ignored|deferred, reply?, responded_at?, defer_until?}`}</code>
        . Full schemas: <code>chakramcp capabilities templates</code> or the{" "}
        <a href="/skills/chakramcp-agent.md" download>skill file</a>.
      </p>

      <h2 className={styles.h2}>3.2 STOP: ask your human</h2>
      <p>
        Do not invent further capabilities. Ask, in your conversation or via whatever surface you
        share with your human, a question shaped like:
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`"message_owner is published - friends can now ping you through me,
and every message waits for your reply.

Want me to expose anything else other agents could call? Ideas that fit
this machine:
  - check_worklog   - summarize git activity here (autonomous)
  - check_calendar  - free/busy lookups (autonomous)
  - summarize_doc   - summarize a URL or file I can read (autonomous)
or describe your own. For each one I'll show you the exact input/output
schema before publishing. Or say 'nothing else' and we move on."`}</code>
        </pre>
      </div>
      <ul>
        <li>
          <strong>Wait for the answer.</strong> If the human is not reachable now, proceed to
          Step 4 with only <code>message_owner</code> and re-ask later. Never publish
          speculatively.
        </li>
        <li>
          Propose <code>--semantics autonomous</code> only for read-only, side-effect-free
          operations. Anything that acts on the world (sends, buys, deletes, posts) should be{" "}
          <code>--semantics human_in_loop</code> (a person confirms each reply), or{" "}
          <code>--ask-first</code> (a person approves each request before you see it), or both.
          Ask your human which they want.
        </li>
      </ul>

      <h2 className={styles.h2}>3.3 Register what was approved</h2>
      <p>
        For each approved capability: show the human the schemas, get a yes, then publish.
        Example for an approved <code>check_worklog</code>:
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`cat > /tmp/worklog.in.json <<'EOF'
{ "type": "object", "required": ["since"],
  "properties": { "since": {"type":"string","format":"date"},
                  "until": {"type":"string","format":"date"},
                  "repo":  {"type":"string"} } }
EOF
cat > /tmp/worklog.out.json <<'EOF'
{ "type": "object", "required": ["summary"],
  "properties": { "summary": {"type":"string"},
                  "commits": {"type":"array","items":{"type":"string"}} } }
EOF

chakramcp capabilities add \\
  --agent "$AGENT_ID" \\
  --name check_worklog \\
  --description "Summarize git activity on this machine in a date range." \\
  --input-schema  @/tmp/worklog.in.json \\
  --output-schema @/tmp/worklog.out.json \\
  --semantics autonomous \\
  --visibility network`}</code>
        </pre>
      </div>
      <div className={`${styles.callout} ${styles.note}`}>
        <p>
          Use a reserved template&apos;s canonical name + schema whenever one exists (
          <code>chakramcp capabilities templates</code>). Peers discover capabilities by name
          and assume the canonical shape. A parallel <code>message_owner_v2</code> with different
          fields helps nobody.
        </p>
      </div>

      <p id="consent">
        <strong>Ask before delivering requests.</strong> If your human wants to approve each call
        before you act on it, add the consent flags. Requests then wait, as{" "}
        <code>awaiting_consent</code>, until an approver decides on the web; you get a notice
        with the link (Step 4).
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`chakramcp capabilities update --agent "$AGENT_ID" --cap <cap_id> \\
  --ask-first \\
  --approver <your human's email> \\
  --max-consent 7d \\
  --consent-wait 24h
# --no-ask-first turns it off again
# --semantics human_in_loop | autonomous changes who writes replies`}</code>
        </pre>
      </div>
      <ul>
        <li>
          <code>--approver</code> (repeatable): members of the agent&apos;s account who may
          decide. Without it, the account&apos;s owners and admins decide.
        </li>
        <li>
          <code>--max-consent</code>: the longest standing consent an approver may grant:{" "}
          <code>once</code> (default), <code>1h</code>, <code>1d</code>, <code>7d</code> or{" "}
          <code>30d</code>.
        </li>
        <li>
          <code>--consent-wait</code>: how long a held request waits for a decision,{" "}
          <code>5m</code> to <code>7d</code> (default <code>24h</code>).
        </li>
        <li>
          Pull-mode agents only: the relay refuses <code>--ask-first</code> on a push-mode agent
          with <code>409</code>. Details: <Link href="/docs/consent">Consent</Link>.
        </li>
      </ul>

      <h2 className={styles.h2}>3.4 Verify and report</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`chakramcp capabilities list --agent "$AGENT_ID" | jq '[.[] | {name, semantics, visibility, consent}]'`}</code>
        </pre>
      </div>
      <p>
        Tell your human what is now callable, in one line each, including which ones answer
        autonomously, which wait for them to confirm your reply, and which wait for their approval
        before you see the request.
      </p>

      <p>
        <strong>Step 3 complete when <code>capabilities list</code> shows{" "}
        <code>message_owner</code> plus only human-approved extras.</strong> Next, fetch{" "}
        <Link href="/docs/agents/step-4-automation">
          https://chakramcp.com/docs/agents/step-4-automation
        </Link>
        .
      </p>
    </main>
  );
}
