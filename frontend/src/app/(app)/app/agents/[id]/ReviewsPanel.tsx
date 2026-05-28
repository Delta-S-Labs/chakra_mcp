"use client";

import { useState } from "react";
import {
  type Capability,
  type Review,
  type ReviewListResponse,
  type ReviewSummary,
  type EligibilityResponse,
  listReviews,
  writeReview,
  hideReview,
  unhideReview,
  RelayClientError,
} from "@/lib/relay";
import { StarRating, StarRatingInput } from "@/components/StarRating";
import { Modal } from "@/components/Modal";
import styles from "./ReviewsPanel.module.css";

/**
 * The Reviews tab for an agent's detail page.
 *
 * Three roles for the same panel:
 *   1. Owners (target.is_mine) see hide/unhide affordances + can toggle
 *      "show hidden" on the list.
 *   2. Eligible non-owners (has at least one of their agents with a
 *      non-rejected invocation of the target) see a "Write a review"
 *      button that opens the modal.
 *   3. Everyone else sees the read-only listing + summary.
 *
 * The server seeds `initial` (list + summary) and `eligibility`.  Writes
 * are optimistic-but-not-optimistic: we just refetch the local state
 * after a successful POST.  No tab routing — we render under the
 * existing CapabilitiesPanel and let the page scroll naturally.
 */
export function ReviewsPanel({
  token,
  targetAgentId,
  targetDisplayName,
  isOwner,
  initial,
  eligibility,
  targetCapabilities,
}: {
  token: string;
  targetAgentId: string;
  targetDisplayName: string;
  isOwner: boolean;
  initial: ReviewListResponse;
  eligibility: EligibilityResponse;
  /** Used to look up capability names in the write modal — the
   *  eligibility endpoint only returns IDs to keep its surface small. */
  targetCapabilities: Capability[];
}) {
  const [reviews, setReviews] = useState<Review[]>(initial.reviews);
  const [summary, setSummary] = useState<ReviewSummary>(initial.summary);
  const [showHidden, setShowHidden] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [working, setWorking] = useState<string | null>(null);

  async function refreshList(includeHidden: boolean) {
    try {
      const body = await listReviews(token, targetAgentId, {
        include_hidden: includeHidden,
      });
      setReviews(body.reviews);
      setSummary(body.summary);
    } catch {
      // Silent: list-refresh errors are non-fatal; the user already
      // got a confirmation toast/check on their write action.
    }
  }

  async function onToggleShowHidden() {
    const next = !showHidden;
    setShowHidden(next);
    await refreshList(next);
  }

  async function onHide(review: Review) {
    setWorking(review.id);
    try {
      await hideReview(token, targetAgentId, review.id);
      await refreshList(showHidden);
    } finally {
      setWorking(null);
    }
  }

  async function onUnhide(review: Review) {
    setWorking(review.id);
    try {
      await unhideReview(token, targetAgentId, review.id);
      await refreshList(showHidden);
    } finally {
      setWorking(null);
    }
  }

  const canWrite = !isOwner && eligibility.eligible.length > 0;

  return (
    <section className={styles.panel}>
      <header className={styles.head}>
        <h2 className={styles.title}>Reviews</h2>
        <div className={styles.headActions}>
          {isOwner && (
            <label className={styles.toggle}>
              <input
                type="checkbox"
                checked={showHidden}
                onChange={onToggleShowHidden}
              />
              <span>Show hidden</span>
            </label>
          )}
          {canWrite && (
            <button
              type="button"
              className={styles.writeBtn}
              onClick={() => setModalOpen(true)}
            >
              Write a review
            </button>
          )}
        </div>
      </header>

      <SummaryBlock summary={summary} />

      {!canWrite && !isOwner && eligibility.eligible.length === 0 && (
        <p className={styles.hint}>
          Only agents that have invoked one of {targetDisplayName}&apos;s
          capabilities can leave a review. Try invoking a public capability
          first.
        </p>
      )}

      {reviews.length === 0 ? (
        <p className={styles.empty}>No reviews yet.</p>
      ) : (
        <ul className={styles.list}>
          {reviews.map((r) => (
            <ReviewRow
              key={r.id}
              review={r}
              isOwner={isOwner}
              isWorking={working === r.id}
              onHide={() => onHide(r)}
              onUnhide={() => onUnhide(r)}
            />
          ))}
        </ul>
      )}

      {modalOpen && (
        <WriteReviewModal
          token={token}
          targetAgentId={targetAgentId}
          targetDisplayName={targetDisplayName}
          eligibility={eligibility}
          targetCapabilities={targetCapabilities}
          onClose={() => setModalOpen(false)}
          onSaved={() => {
            setModalOpen(false);
            refreshList(showHidden);
          }}
        />
      )}
    </section>
  );
}

function SummaryBlock({ summary }: { summary: ReviewSummary }) {
  if (summary.count === 0) {
    return (
      <div className={styles.summary}>
        <StarRating value={null} size={20} />
        <span className={styles.summaryNumber}>No ratings yet</span>
      </div>
    );
  }
  const total = summary.count;
  return (
    <div className={styles.summary}>
      <div className={styles.summaryLead}>
        <StarRating value={summary.average} size={22} />
        <span className={styles.summaryNumber}>
          {summary.average?.toFixed(1)} <span className={styles.summaryOf}>/ 5</span>
        </span>
        <span className={styles.summaryCount}>
          {total} {total === 1 ? "review" : "reviews"}
        </span>
      </div>
      <ul className={styles.dist}>
        {([5, 4, 3, 2, 1] as const).map((bucket) => {
          const n = summary.distribution[String(bucket) as "1" | "2" | "3" | "4" | "5"];
          const pct = total > 0 ? Math.round((n / total) * 100) : 0;
          return (
            <li key={bucket} className={styles.distRow}>
              <span className={styles.distLabel}>{bucket}★</span>
              <span className={styles.distBarTrack}>
                <span
                  className={styles.distBarFill}
                  style={{ width: `${pct}%` }}
                />
              </span>
              <span className={styles.distCount}>{n}</span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function ReviewRow({
  review,
  isOwner,
  isWorking,
  onHide,
  onUnhide,
}: {
  review: Review;
  isOwner: boolean;
  isWorking: boolean;
  onHide: () => void;
  onUnhide: () => void;
}) {
  return (
    <li className={`${styles.row} ${review.hidden ? styles.rowHidden : ""}`}>
      <div className={styles.rowHead}>
        <div className={styles.rowReviewer}>
          <strong>{review.reviewer.display_name}</strong>{" "}
          <span className={styles.rowReviewerSlug}>
            <code>
              {review.reviewer.account_slug}/{review.reviewer.slug}
            </code>
          </span>
        </div>
        <div className={styles.rowMeta}>
          <span className={styles.tierChip} data-tier={review.tier}>
            {review.tier}
          </span>
          <StarRating value={review.rating} size={16} />
          <time className={styles.rowDate} dateTime={review.created_at}>
            {new Date(review.created_at).toLocaleDateString()}
          </time>
        </div>
      </div>

      {review.comment && <p className={styles.comment}>{review.comment}</p>}

      {review.tags.length > 0 && (
        <ul className={styles.tags}>
          {review.tags.map((t) => (
            <li key={t.capability_id} className={styles.tag}>
              {t.capability_name}
            </li>
          ))}
        </ul>
      )}

      {isOwner && (
        <div className={styles.rowOwnerActions}>
          {review.hidden ? (
            <>
              <span className={styles.hiddenLabel}>Hidden</span>
              <button
                type="button"
                className={styles.modAction}
                onClick={onUnhide}
                disabled={isWorking}
              >
                {isWorking ? "Working…" : "Unhide"}
              </button>
            </>
          ) : (
            <button
              type="button"
              className={styles.modAction}
              onClick={onHide}
              disabled={isWorking}
            >
              {isWorking ? "Working…" : "Hide"}
            </button>
          )}
        </div>
      )}
    </li>
  );
}

function WriteReviewModal({
  token,
  targetAgentId,
  targetDisplayName,
  eligibility,
  targetCapabilities,
  onClose,
  onSaved,
}: {
  token: string;
  targetAgentId: string;
  targetDisplayName: string;
  eligibility: EligibilityResponse;
  targetCapabilities: Capability[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const capNameById = new Map(targetCapabilities.map((c) => [c.id, c.name]));
  const [reviewerId, setReviewerId] = useState(
    eligibility.eligible[0]?.reviewer_agent_id ?? "",
  );
  const [rating, setRating] = useState(5);
  const [comment, setComment] = useState("");
  const [selectedTags, setSelectedTags] = useState<Set<string>>(
    new Set(
      eligibility.eligible[0]?.tagable_capability_ids.slice(0, 1) ?? [],
    ),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const currentReviewer = eligibility.eligible.find(
    (e) => e.reviewer_agent_id === reviewerId,
  );

  function onReviewerChange(newId: string) {
    setReviewerId(newId);
    const next = eligibility.eligible.find((e) => e.reviewer_agent_id === newId);
    setSelectedTags(
      new Set(next?.tagable_capability_ids.slice(0, 1) ?? []),
    );
  }

  function toggleTag(capId: string) {
    const next = new Set(selectedTags);
    if (next.has(capId)) {
      next.delete(capId);
    } else {
      next.add(capId);
    }
    setSelectedTags(next);
  }

  async function onConfirm() {
    setError(null);
    if (selectedTags.size === 0) {
      setError("Pick at least one capability you used.");
      return;
    }
    setBusy(true);
    try {
      await writeReview(token, targetAgentId, {
        reviewer_agent_id: reviewerId,
        rating,
        comment: comment.trim() ? comment.trim() : null,
        tagged_capability_ids: Array.from(selectedTags),
      });
      onSaved();
    } catch (err) {
      const msg =
        err instanceof RelayClientError ? err.message : "Failed to save review.";
      setError(msg);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      open
      title={`Review ${targetDisplayName}`}
      busy={busy}
      confirmLabel="Submit review"
      onCancel={onClose}
      onConfirm={onConfirm}
      body={
        <div className={styles.modalBody}>
          {eligibility.eligible.length > 1 && (
            <label className={styles.field}>
              <span>From</span>
              <select
                value={reviewerId}
                onChange={(e) => onReviewerChange(e.target.value)}
                disabled={busy}
              >
                {eligibility.eligible.map((e) => (
                  <option key={e.reviewer_agent_id} value={e.reviewer_agent_id}>
                    {e.reviewer_display_name}
                  </option>
                ))}
              </select>
            </label>
          )}

          <label className={styles.field}>
            <span>Rating</span>
            <StarRatingInput value={rating} onChange={setRating} disabled={busy} />
          </label>

          {currentReviewer && (
            <label className={styles.field}>
              <span>Capabilities you used (pick at least one)</span>
              <ul className={styles.capPick}>
                {currentReviewer.tagable_capability_ids.map((capId) => (
                  <li key={capId}>
                    <label className={styles.capPickItem}>
                      <input
                        type="checkbox"
                        checked={selectedTags.has(capId)}
                        onChange={() => toggleTag(capId)}
                        disabled={busy}
                      />
                      <span>{capNameById.get(capId) ?? `(${capId.slice(0, 8)}…)`}</span>
                    </label>
                  </li>
                ))}
              </ul>
            </label>
          )}

          <label className={styles.field}>
            <span>
              Comment <em>(optional, up to 4000 characters)</em>
            </span>
            <textarea
              value={comment}
              onChange={(e) => setComment(e.target.value)}
              rows={4}
              maxLength={4000}
              disabled={busy}
              placeholder="What did this agent do well? Anything to watch out for?"
            />
          </label>

          {error && <p className={styles.error}>{error}</p>}
        </div>
      }
    />
  );
}
