import Link from "next/link";
import { notFound } from "next/navigation";
import { auth } from "@/auth";
import {
  getAgent,
  listCapabilities,
  listReviews,
  getReviewEligibility,
  RelayClientError,
  type EligibilityResponse,
  type ReviewListResponse,
} from "@/lib/relay";
import { StarRating } from "@/components/StarRating";
import { EditAgentForm } from "./EditAgentForm";
import { CapabilitiesPanel } from "./CapabilitiesPanel";
import { ReviewsPanel } from "./ReviewsPanel";
import styles from "../agents.module.css";

export default async function AgentDetailPage({
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

  let agent: Awaited<ReturnType<typeof getAgent>>;
  try {
    agent = await getAgent(token, id);
  } catch (err) {
    if (err instanceof RelayClientError && err.status === 404) {
      notFound();
    }
    throw err;
  }

  let capabilities: Awaited<ReturnType<typeof listCapabilities>> = [];
  try {
    capabilities = await listCapabilities(token, id);
  } catch {
    // non-fatal - show the header anyway
  }

  // Reviews + eligibility are non-fatal too — show the rest of the page
  // even if these requests blow up.
  let reviews: ReviewListResponse = {
    reviews: [],
    summary: {
      average: null,
      count: 0,
      distribution: { "1": 0, "2": 0, "3": 0, "4": 0, "5": 0 },
    },
    next_cursor: null,
  };
  try {
    reviews = await listReviews(token, id, {
      include_hidden: agent.is_mine ? true : false,
    });
  } catch {
    // non-fatal
  }

  let eligibility: EligibilityResponse = { eligible: [] };
  // The eligibility endpoint is only meaningful when the caller is
  // NOT the target's owner (you can't review your own agent). Skip the
  // call for owners — saves a round-trip + avoids the trivially-empty
  // payload.
  if (!agent.is_mine) {
    try {
      eligibility = await getReviewEligibility(token, id);
    } catch {
      // non-fatal — modal won't open, but the read-only panel still renders
    }
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href="/app/agents" className={styles.backLink}>
            ← Agents
          </Link>
        </div>
        <h1 className={styles.title}>
          {agent.display_name}{" "}
          <span
            className={`${styles.badge} ${
              agent.visibility === "network" ? styles.badgeOn : ""
            }`}
          >
            {agent.visibility}
          </span>
        </h1>
        <p className={styles.body}>
          <code>
            {agent.account_slug}/{agent.slug}
          </code>{" "}
          · owned by <strong>{agent.account_display_name}</strong>
        </p>
        {agent.review_count > 0 && (
          <p className={styles.body}>
            <StarRating value={agent.avg_rating} size={16} showNumber /> ·{" "}
            <span style={{ color: "var(--ink-soft)" }}>
              {agent.review_count}{" "}
              {agent.review_count === 1 ? "review" : "reviews"}
            </span>
          </p>
        )}
        {agent.description && <p className={styles.body}>{agent.description}</p>}
      </header>

      {agent.is_mine ? (
        <EditAgentForm token={token} agent={agent} />
      ) : (
        <div className={styles.notice}>
          You&apos;re viewing this agent as a network visitor. Editing is
          limited to members of <strong>{agent.account_display_name}</strong>.
        </div>
      )}

      <CapabilitiesPanel
        token={token}
        agentId={agent.id}
        canEdit={agent.is_mine}
        initial={capabilities}
      />

      <ReviewsPanel
        token={token}
        targetAgentId={agent.id}
        targetDisplayName={agent.display_name}
        isOwner={agent.is_mine}
        initial={reviews}
        eligibility={eligibility}
        targetCapabilities={capabilities}
      />
    </div>
  );
}
