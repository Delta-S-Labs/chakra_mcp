# Agent Ratings & Reviews — Implementation Plan

Derived from `2026-05-28-agent-ratings-and-reviews-design.md`. Same PR-shaped sequencing as
sub-project 1 — backend first, the rest depends only on the API contract that lands here.

## PR 1 — Backend: schema + endpoints + aggregate exposure + tests

The cohesive backend slice. Everything ships together.

### Phase 1.1 — Migration `0023_agent_ratings_and_reviews.sql`
- `agent_reviews` table with the columns + CHECKs + unique pair + indexes per spec.
- `agent_review_tags` join table.
- Apply locally, sqlx-prepare.
- **Verify:** `\d agent_reviews` shows columns + CHECKs; manual INSERTs reject (rating 6,
  self-review, duplicate pair).

### Phase 1.2 — Reviews handler (relay `handlers/reviews.rs`, new file)
- `ReviewDto` (id, reviewer + target `AgentSummary`, rating, comment, tier, tagged
  capability ids + names, hidden flag, created/updated).
- `WriteReviewRequest`, `ReviewListResponse` (with `summary` block).
- `POST /v1/agents/{target}/reviews` — full validation order from the spec; tier resolution;
  upsert + tag swap in a transaction.
- `GET /v1/agents/{target}/reviews` — cursor pagination, tier filter, `include_hidden`
  member-only, summary computation in the same query (`SUM` + `COUNT FILTER`).
- `POST .../reviews/{id}/hide` and `…/unhide`.
- `GET /v1/agents/{target}/reviews/eligibility?reviewer=...` — returns the
  `(reviewer_agent_id → tagable_capability_ids)` tuples for the form.
- Wire routes in `lib.rs`.

### Phase 1.3 — Aggregate exposure on existing endpoints
- `DiscoveryAgent` + `AgentDto` gain `avg_rating: Option<f64>` + `review_count: i64`. SQL adds
  two correlated subqueries on `agent_reviews … WHERE hidden_at IS NULL`. Composite index
  `(target_agent_id, hidden_at) WHERE hidden_at IS NULL` (the migration created it under a
  different name; re-use).

### Phase 1.4 — Tests (`sqlx::test`)
Cover the test list from the spec verbatim: happy paths (friend + public), tag validation
(wrong-agent / not-invoked / empty), self-review + rating-bounds + ownership 403s, upsert
semantics (rating/comment/tags swap, `created_at` preserved, `updated_at` bumped), soft-hide
(member-only, aggregate exclusion, idempotent unhide), aggregate correctness, eligibility
endpoint.

### Phase 1.5 — Sanity
- All relay tests green.
- sqlx-prepare cache regenerated, fmt + clippy clean.
- Open PR, admin-merge once CI green.

## PR 2 — Frontend
- `lib/relay.ts`: `Review`, `WriteReviewRequest`, `ReviewListResponse`,
  `ReviewEligibilityResponse` types + client helpers
  (`listReviews`, `writeReview`, `hideReview`, `unhideReview`, `getReviewEligibility`).
- `Agent` + `DiscoveryAgent` types widen with `avg_rating` + `review_count`.
- **Public directory** (`/agents`): card renders `★ 4.6 · 38` chip when `review_count > 0`.
- **Agent detail page** (`/app/agents/[id]`): new `ReviewsSection` component with summary
  (avg + count + distribution bar), `WriteReviewForm` (reviewer-agent picker, rating
  selector, comment, multi-select of tagable capabilities driven by the eligibility endpoint),
  paginated list with tier badges + Hide button for owners.
- tsc + eslint clean; preview-verify happy path + hide.

## PR 3 — CLI
- `chakramcp reviews list <target>` / `write <target> --as --rating --comment --tag …` /
  `hide <id>` / `unhide <id>`.
- `--tag` repeatable.
- Verify against dev backend; `cargo clippy` clean.

## PR 4 — SDKs (Py + TS + Rust + Go)
- `Review` type + `ReviewsClient` with `list / write / hide / unhide / eligibility`.
- Typed `CapabilityNotInvokedError` (subclass of the generic API error) for the `400 invalid_request`
  with code `capability_not_invoked`. Same routing pattern as `QuotaExhaustedError` from
  sub-project 1 PR4.
- Build + tests for each SDK.

## Releases (post-merge)
Same flow as last time:
- CLI: bump `backend/cli/Cargo.toml`, push `cli-v0.1.5` tag.
- Python SDK: bump `src/chakramcp/__init__.py`, push `sdk-py-v0.3.2`.
- TypeScript SDK: bump `sdks/typescript/package.json`, push `sdk-ts-v0.3.2`.
- Rust/Go SDKs publish via their own flows.

Per the user's directive, these releases ship **after sub-project 2 lands** and cover both
sub-projects (public-invoke + ratings) in one user-facing release wave.

## Sequencing notes

- PR 1 is the critical path; PRs 2–4 parallelize on its contract.
- Migration 0023 is additive and dark; no backfill, nothing is rendered on existing agents
  until reviews accumulate.
- The "public reviews" tier is *runtime-dependent* on sub-project 1 (#126), already on `main`.
  No build-time coupling.
