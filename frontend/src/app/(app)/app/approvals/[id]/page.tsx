import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { auth } from "@/auth";
import { describeTerms, inputFields, timeLeft } from "@/lib/consent";
import { getApproval, RelayClientError, type ApprovalView } from "@/lib/relay";
import { redirectIfUnauthorized } from "@/lib/session-expired";
import { LocalTime } from "../../audit/LocalTime";
import { DecisionForm } from "./DecisionForm";
import { ReplyForm } from "./ReplyForm";
import styles from "../approvals.module.css";

export const metadata: Metadata = {
  title: "Approval · ChakraMCP",
};

/**
 * /app/approvals/[id] — the link an agent passes its owner when a call to
 * it waits for approval (owner consent). Laid out like an Account
 * Aggregator consent screen: who's asking, for what, why, what they're
 * sending, for how long, and until when to decide.
 */
export default async function ApprovalPage({
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

  let view: ApprovalView;
  try {
    view = await getApproval(token, id);
  } catch (err) {
    redirectIfUnauthorized(err, `/app/approvals/${id}`);
    if (err instanceof RelayClientError && err.status === 404) notFound();
    if (err instanceof RelayClientError && err.status === 403) {
      return (
        <div className={styles.page}>
          <p className={styles.error}>
            Approvals need you signed in to the web app as yourself. Sign out and in again,
            then open the link.
          </p>
        </div>
      );
    }
    throw err;
  }

  const expired = view.status === "pending" && new Date(view.expires_at) <= new Date();
  const pending = view.status === "pending" && !expired;
  const requester = view.requester.agent_display_name ?? "An agent";
  const capability = view.capability.name ?? "a deleted capability";
  const fields = inputFields(view.input);
  const isReply = view.kind === "reply";
  const title = isReply
    ? pending
      ? "Confirm a reply?"
      : "A reply from your agent"
    : pending
      ? "Approve a request?"
      : "A request to your agent";

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href="/app/approvals">Approvals</Link>
        </div>
        <h1 className={styles.title}>{title}</h1>
        {isReply ? (
          <p className={styles.lede}>
            Your agent <strong>{view.target.display_name}</strong> drafted a reply to{" "}
            <strong>{requester}</strong>
            {view.requester.verified && <span className={styles.verified}>Verified</span>} for{" "}
            <strong>{capability}</strong>.
            {pending ? " It goes out only once you confirm it." : null}
          </p>
        ) : (
          <p className={styles.lede}>
            <strong>{requester}</strong>
            {view.requester.verified && <span className={styles.verified}>Verified</span>} asks to
            use <strong>{capability}</strong> on your agent <strong>{view.target.display_name}</strong>.
          </p>
        )}
      </header>

      <section className={styles.card} aria-label="The request">
        <dl className={styles.facts}>
          <dt>Who&apos;s asking</dt>
          <dd>
            {requester}{" "}
            {view.requester.account_display_name && (
              <span className={styles.muted}>{view.requester.account_display_name}</span>
            )}
          </dd>
          <dt>What</dt>
          <dd>
            <code>{capability}</code>
            {view.capability.description && (
              <>
                <br />
                <span className={styles.muted}>{view.capability.description}</span>
              </>
            )}
          </dd>
          {view.terms.purpose && (
            <>
              <dt>Why</dt>
              <dd>{view.terms.purpose}</dd>
            </>
          )}
          {view.grant_purpose && (
            <>
              <dt>Your grant says</dt>
              <dd>{view.grant_purpose}</dd>
            </>
          )}
          {!isReply && (
            <>
              <dt>For how long</dt>
              <dd>{describeTerms(view.terms)}</dd>
            </>
          )}
          <dt>{pending ? "Decide by" : "Asked"}</dt>
          <dd>
            <LocalTime iso={pending ? view.expires_at : view.created_at} />
            {pending && <span className={styles.muted}> · {timeLeft(view.expires_at)}</span>}
          </dd>
        </dl>
      </section>

      {fields.length > 0 && (
        <section className={styles.card} aria-label="What they're sending">
          <h2 className={styles.cardTitle}>
            {isReply ? "What they asked" : "What they\u2019re sending"}
          </h2>
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

      {isReply && view.draft && (
        <section className={styles.card} aria-label="The draft reply">
          <h2 className={styles.cardTitle}>
            {view.draft.status === "failed" ? "Your agent's reply: it couldn't" : "Your agent's reply"}
          </h2>
          {view.draft.status === "failed" ? (
            <p className={styles.outcome}>{view.draft.error ?? "Failed"}</p>
          ) : (
            <dl className={styles.facts}>
              {inputFields(view.draft.output).map((f) => (
                <div key={f.name} style={{ display: "contents" }}>
                  <dt>{f.name}</dt>
                  <dd>
                    <code>{f.value}</code>
                  </dd>
                </div>
              ))}
            </dl>
          )}
        </section>
      )}

      {pending && view.may_decide && isReply && view.draft_sha256 && (
        <ReplyForm id={view.id} draftSha256={view.draft_sha256} />
      )}
      {pending && view.may_decide && !isReply && (
        <DecisionForm id={view.id} terms={view.terms} maxValidity={view.max_validity} />
      )}
      {pending && !view.may_decide && (
        <p className={styles.outcome}>
          Only{" "}
          {view.approvers.length
            ? view.approvers.map((a) => a.display_name).join(", ")
            : "the account's owners and admins"}{" "}
          can decide on this request.
        </p>
      )}
      {!pending && <p className={styles.outcome}>{outcome(view, expired)}</p>}
    </div>
  );
}

/** What happened to a decided request, in a sentence. */
function outcome(view: ApprovalView, expired: boolean): React.ReactNode {
  const at = view.decided_at ? <LocalTime iso={view.decided_at} /> : null;
  const note = view.decision_note ? ` Note: ${view.decision_note}` : "";
  switch (view.status) {
    case "approved":
      if (view.kind === "reply") {
        return <>Confirmed and sent {at && <>on {at}</>}.</>;
      }
      return (
        <>
          Approved {at && <>on {at}</>}: {describeTerms(view.approved_terms).toLowerCase()}.
        </>
      );
    case "denied":
      return (
        <>
          {view.kind === "reply" ? "Rejected" : "Denied"} {at && <>on {at}</>}.{note}
        </>
      );
    case "cancelled":
      return (
        <>
          Cancelled {at && <>on {at}</>}.{note}
        </>
      );
    default:
      if (expired || view.status === "expired") {
        return view.kind === "reply"
          ? "Nobody confirmed this reply in time, so the call failed."
          : "This request expired before anyone decided.";
      }
      return `This request is ${view.status}.`;
  }
}
