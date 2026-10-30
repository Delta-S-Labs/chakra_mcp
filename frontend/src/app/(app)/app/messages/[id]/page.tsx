import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { auth } from "@/auth";
import { inputFields, timeLeft } from "@/lib/consent";
import { getMessage, RelayClientError, type MessageView } from "@/lib/relay";
import { redirectIfUnauthorized } from "@/lib/session-expired";
import { LocalTime } from "../../audit/LocalTime";
import { ReplyBox } from "./ReplyBox";
import styles from "../../approvals/approvals.module.css";

export const metadata: Metadata = {
  title: "Message · ChakraMCP",
};

/**
 * /app/messages/[id] — the link an agent passes its owner for a call to a
 * `human_in_loop` capability (`owner_reply_url`). One of the agent's
 * approvers answers it here, whether or not the agent claimed it; the
 * answer goes straight back to the caller.
 */
export default async function MessagePage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const session = await auth();
  const token = session?.backendToken ?? null;
  if (!token) {
    return <div className={styles.error}>Sign in again - no backend token in this session.</div>;
  }

  let view: MessageView;
  try {
    view = await getMessage(token, id);
  } catch (err) {
    redirectIfUnauthorized(err, `/app/messages/${id}`);
    if (err instanceof RelayClientError && err.status === 404) notFound();
    if (err instanceof RelayClientError && err.status === 403) {
      return (
        <div className={styles.page}>
          <p className={styles.error}>
            Answering needs you signed in to the web app as yourself. Sign out and in again,
            then open the link.
          </p>
        </div>
      );
    }
    throw err;
  }

  const requester = view.requester ?? "An agent";
  const fields = inputFields(view.input);
  const draftHref = view.draft_approve_url
    ? new URL(view.draft_approve_url, "http://x").pathname
    : null;

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href="/app/approvals">Approvals</Link>
        </div>
        <h1 className={styles.title}>
          {view.answerable ? "Answer a message?" : "A message to your agent"}
        </h1>
        <p className={styles.lede}>
          <strong>{requester}</strong>
          {view.requester_verified && <span className={styles.verified}>Verified</span>} called{" "}
          <strong>{view.capability_name}</strong> on your agent <strong>{view.target}</strong>.
          {view.answerable ? " Your answer goes straight back to them." : null}
        </p>
      </header>

      <section className={styles.card} aria-label="The message">
        <dl className={styles.facts}>
          <dt>Who&apos;s asking</dt>
          <dd>
            {requester}{" "}
            {view.requester_account && (
              <span className={styles.muted}>{view.requester_account}</span>
            )}
          </dd>
          <dt>What</dt>
          <dd>
            <code>{view.capability_name}</code>
            {view.capability_description && (
              <>
                <br />
                <span className={styles.muted}>{view.capability_description}</span>
              </>
            )}
          </dd>
          <dt>Sent</dt>
          <dd>
            <LocalTime iso={view.created_at} />
          </dd>
          {view.answerable && view.expires_at && (
            <>
              <dt>Answer by</dt>
              <dd>
                <LocalTime iso={view.expires_at} />
                <span className={styles.muted}> · {timeLeft(view.expires_at)}</span>
              </dd>
            </>
          )}
        </dl>
      </section>

      {fields.length > 0 && (
        <section className={styles.card} aria-label="What they sent">
          <h2 className={styles.cardTitle}>What they sent</h2>
          <dl className={styles.facts}>
            {fields.map((f) => (
              <div key={f.name} style={{ display: "contents" }}>
                <dt>{f.name}</dt>
                <dd>
                  <code>{f.value}</code>
                </dd>
              </div>
            ))}
          </dl>
        </section>
      )}

      {view.answerable ? (
        <ReplyBox id={view.invocation_id} shape={view.reply_shape} />
      ) : draftHref ? (
        <p className={styles.outcome}>
          Your agent drafted a reply. <Link href={draftHref}>Confirm or reject it</Link>.
        </p>
      ) : (
        <p className={styles.outcome}>{outcome(view)}</p>
      )}
    </div>
  );
}

/** How a call that can't be answered here ended, in a sentence. */
function outcome(view: MessageView): string {
  switch (view.status) {
    case "succeeded":
      return "It's been answered.";
    case "failed":
      return `It was declined${view.error_message ? `: ${view.error_message}` : ""}.`;
    case "timeout":
      return "Nobody answered it in time, so it timed out.";
    case "rejected":
      return "It was refused.";
    case "awaiting_consent":
      return "It's waiting for your approval first.";
    default:
      return `It's ${view.status}.`;
  }
}
