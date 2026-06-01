#!/usr/bin/env python3
"""
ChakraMCP demo-data seeder.

Populates a single user's account on a ChakraMCP relay with enough
agents / capabilities / friendships / grants / invocations / reviews
that the directory, friendships page, audit log, and agent-detail
review summaries all render with realistic numbers — good for
screenshots, screencasts, and "kick the tires" walkthroughs.

Auth: takes one API key from `CHAKRAMCP_API_KEY`. Everything is
created under whichever user that key belongs to.

Surface coverage:
  - Creates 2 org accounts (in addition to the user's individual one)
  - Creates 25 agents across the 3 accounts
  - Creates 40 capabilities (mix of friendship-only + public_invoke=true)
  - Proposes 50 friendships (40 accepted, 6 left proposed/pending, 4 rejected)
  - Issues 25 grants (20 active, 5 revoked)
  - Runs ~150 real invocations (succeeded / failed / rejected) by
    calling /v1/invoke as the grantee then /v1/invocations/{id}/result
    as the granter — both with the same API key (the caller owns
    every agent).
  - Writes 80 reviews with a 1-5★ spread (4.x average), comments on
    ~70% of them, ~75% friend tier / ~25% public tier.

Usage:
    pip install chakramcp-sdk httpx
    CHAKRAMCP_API_KEY=ck_... python seed.py [--scale heavy|light|minimal]
                                            [--app-url ...] [--relay-url ...]

The script is idempotent on creates: 409/409-ish conflicts (slug
exists, friendship pair exists, review pair exists) are swallowed
so you can re-run safely. Each re-run adds more invocations + some
new reviews if there's headroom under the one-per-pair limit.

Limitation: the relay records `created_at = now()` on invocations
and reviews — you can't backdate via the API. Time-series charts
will show today's date only. For multi-day demo data, run the
seeder again on consecutive days (re-runs accumulate).

Cross-account caveat: since the API key belongs to one user, ALL
agents are owned by that user. The relay tags `is_mine=true` and
`i_authored=true` accordingly on the dashboard. To demo
multi-tenant interactions, manually create a second user account
and re-run from each.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import sys
import time
from dataclasses import dataclass, field
from typing import Any

import httpx

# We import the published SDK at runtime so this script doubles as a
# "look, the SDK works end-to-end" demo.
try:
    from chakramcp import ChakraMCP, ChakraMCPError
except ImportError:
    print("Please `pip install chakramcp-sdk httpx` first.", file=sys.stderr)
    sys.exit(1)


# ─── Seeding plans (light / heavy / minimal) ───────────────────────

@dataclass
class Plan:
    orgs: int
    agents: int
    capabilities: int
    public_invoke_caps: int
    friendships: int
    accepted_pct: float
    rejected_pct: float
    grants: int
    revoked_grants: int
    invocations: int
    reviews: int

PLANS = {
    "minimal": Plan(
        orgs=1, agents=3, capabilities=4, public_invoke_caps=1,
        friendships=2, accepted_pct=1.0, rejected_pct=0.0,
        grants=2, revoked_grants=0, invocations=5, reviews=2,
    ),
    "light": Plan(
        orgs=2, agents=10, capabilities=15, public_invoke_caps=5,
        friendships=12, accepted_pct=0.85, rejected_pct=0.10,
        grants=6, revoked_grants=1, invocations=80, reviews=25,
    ),
    "heavy": Plan(
        orgs=2, agents=25, capabilities=40, public_invoke_caps=15,
        friendships=50, accepted_pct=0.80, rejected_pct=0.08,
        grants=25, revoked_grants=5, invocations=150, reviews=80,
    ),
}


# ─── Naming pools ──────────────────────────────────────────────────

ORG_NAMES = [
    ("acme-labs",       "Acme Labs"),
    ("trip-co",         "Trip Co"),
    ("helios-research", "Helios Research"),
]

# Mythology + tools — pretty + distinct.
AGENT_NAMES = [
    ("hermes",     "Hermes",           "Message routing + general coordination"),
    ("iris",       "Iris",             "Inbox triage & smart-summarisation"),
    ("athena",     "Athena",           "Strategic planning + meeting scheduling"),
    ("apollo",     "Apollo",           "Documentation Q&A across internal wikis"),
    ("artemis",    "Artemis",          "Calendar agent — books slots across timezones"),
    ("morpheus",   "Morpheus",         "Sleep-cycle aware standup digest"),
    ("orpheus",    "Orpheus",          "Music + ambience suggestion engine"),
    ("nyx",        "Nyx",              "After-hours pager / on-call hand-off"),
    ("kairos",     "Kairos",           "Deadline + SLA tracker"),
    ("janus",      "Janus",            "API contract validator (input/output)"),
    ("vulcan",     "Vulcan",           "Build + deploy orchestrator"),
    ("oracle",     "Oracle",           "Internal data Q&A"),
    ("phoenix",    "Phoenix",          "Incident response runbook executor"),
    ("atlas",      "Atlas",            "Map + geolocation queries"),
    ("neptune",    "Neptune",          "Travel booking — flights + hotels"),
    ("mercury",    "Mercury",          "Currency conversion + market quotes"),
    ("venus",      "Venus",            "Image generation router"),
    ("mars",       "Mars",             "Marketing campaign drafting"),
    ("ceres",      "Ceres",            "Catering + food delivery agent"),
    ("juno",       "Juno",             "HR & people-ops Q&A"),
    ("vesta",      "Vesta",            "Office-management ops"),
    ("minerva",    "Minerva",          "Code review + lint summariser"),
    ("diana",      "Diana",            "Talent / candidate evaluation"),
    ("bacchus",    "Bacchus",          "Event-planning assistant"),
    ("saturn",     "Saturn",           "Financial reporting"),
    ("cupid",      "Cupid",            "Customer-success engagement notes"),
    ("flora",      "Flora",            "Plant-care reminder bot (yes, really)"),
    ("luna",       "Luna",             "Night-mode test runner"),
]

# A capability schema we'll reuse — input is freeform JSON, output
# is a single-string echo. Real services would have meaningful
# Schemas; the relay only inspects them for validity.
DEFAULT_INPUT_SCHEMA = {
    "type": "object",
    "properties": {"text": {"type": "string"}},
    "required": ["text"],
}
DEFAULT_OUTPUT_SCHEMA = {
    "type": "object",
    "properties": {"reply": {"type": "string"}},
    "required": ["reply"],
}

CAPABILITY_NAMES = [
    ("schedule_meeting",     "Find a slot and book it"),
    ("summarize_thread",     "Condense a slack/email thread to bullets"),
    ("translate_text",       "Translate text between languages"),
    ("book_table",           "Reserve a restaurant table"),
    ("look_up_flight",       "Cheapest flight on a route + day"),
    ("draft_email",          "Generate an email draft from bullets"),
    ("score_resume",         "Rate a resume against a JD"),
    ("monitor_status",       "Poll a status endpoint, alert on change"),
    ("run_query",            "Execute a parameterised data query"),
    ("send_pager",           "Page on-call about an incident"),
    ("classify_intent",      "Classify a user message's intent"),
    ("draft_changelog",      "Write a changelog entry from a PR diff"),
    ("transcribe_audio",     "Transcribe an audio clip"),
    ("synthesize_speech",    "Read text aloud (TTS)"),
    ("detect_pii",           "Flag personal data in text"),
    ("price_compare",        "Compare prices across vendors"),
    ("estimate_eta",         "Estimate arrival time"),
    ("plan_itinerary",       "Plan a multi-day trip"),
    ("review_pr",            "Summarise diff + flag risks"),
    ("score_lead",           "Score a sales lead against ICP"),
    ("draft_brief",          "Draft a strategy brief"),
    ("propose_agenda",       "Propose a meeting agenda"),
    ("clip_highlight",       "Extract highlights from a video"),
    ("compose_post",         "Draft a social-media post"),
    ("research_topic",       "Web-search + summarise a topic"),
    ("score_npr",            "Score an NPR podcast for relevance"),
    ("alert_calendar",       "Check calendar conflicts"),
    ("score_intent",         "Score buying intent on a lead"),
    ("rate_feedback",        "Rate customer feedback sentiment"),
    ("draft_release_notes",  "Compose release notes from PRs"),
    ("plan_party",           "Plan an event start-to-finish"),
    ("find_speaker",         "Find a conference speaker"),
    ("draft_thank_you",      "Draft a thank-you note"),
    ("score_blog_post",      "Critique a blog draft"),
    ("propose_offer",        "Propose a deal price"),
    ("draft_jd",             "Draft a job description"),
    ("score_estimate",       "Sanity-check an engineering estimate"),
    ("compose_summary",      "Compose an end-of-week summary"),
    ("classify_priority",    "Rank items by priority"),
    ("score_relevance",      "Rank items by relevance to a query"),
]


# Review-prompt corpora — one per star bucket. We blend these with
# the capability name to look natural in the demo UI.
REVIEW_COMMENTS = {
    5: [
        "Saved me an hour. Output was bang-on, no follow-up needed.",
        "Genuinely surprised — first try, exactly what I needed.",
        "Best of the agents I've tried for this. Easy 5.",
        "Drop-in replacement for a contractor who used to do this for us.",
        "Fast, accurate, and the JSON shape is sane. Would happily auto-invoke.",
        "",
    ],
    4: [
        "Solid. Two of ten outputs needed a light edit, the rest shipped as-is.",
        "Reliable for the common case. Edge cases sometimes need a second pass.",
        "Good — knock off a star because the latency varies a bit.",
        "Works. I'd grant more capability to this one over time.",
        "",
    ],
    3: [
        "It works but the response shape changes between calls. Annoying to integrate.",
        "Middle of the road. Useful when I'm in a hurry, less so for anything important.",
        "Half the time great, half the time generic. Variance is too high for autonomous use.",
        "Output technically correct but lacks the structure I need.",
        "",
    ],
    2: [
        "Misunderstood what I asked for the first three times. Eventually got it.",
        "Tone is off. Skip this one for anything customer-facing.",
        "The output looked good but a column was hallucinated. Needs supervision.",
        "Slow + occasionally returns 4xx. Not ready for production.",
        "",
    ],
    1: [
        "Returned a malformed payload, twice. Avoid.",
        "Wouldn't accept the input format the README documents. Wasted an hour.",
        "Hallucinated content + presented it as fact. Hard pass.",
        "Completely off-base. Couldn't get it to do the basic thing.",
        "",
    ],
}


# ─── Bookkeeping ───────────────────────────────────────────────────

@dataclass
class Seeded:
    """All the resources we created (or discovered already existed)."""
    orgs: list[dict[str, Any]] = field(default_factory=list)
    agents: list[dict[str, Any]] = field(default_factory=list)
    capabilities: list[dict[str, Any]] = field(default_factory=list)
    friendships: list[dict[str, Any]] = field(default_factory=list)
    grants: list[dict[str, Any]] = field(default_factory=list)
    invocations: list[dict[str, Any]] = field(default_factory=list)
    reviews: list[dict[str, Any]] = field(default_factory=list)


def info(msg: str, *, indent: int = 0) -> None:
    print(f"{'  ' * indent}{msg}", flush=True)


# ─── Step 1: ensure orgs ───────────────────────────────────────────

def ensure_orgs(http: httpx.Client, app_url: str, plan: Plan) -> list[dict[str, Any]]:
    """Make sure the user is a member of `plan.orgs` org accounts.

    Idempotent: if the slug already exists, the 409 is swallowed and
    the existing row is fetched.
    """
    info(f"Step 1: ensuring {plan.orgs} org account(s) exist…")
    out: list[dict[str, Any]] = []

    # First, find what's already there.
    r = http.get(f"{app_url}/v1/orgs")
    r.raise_for_status()
    existing_by_slug = {o["slug"]: o for o in r.json() if o["account_type"] == "organization"}

    for slug, display_name in ORG_NAMES[:plan.orgs]:
        if slug in existing_by_slug:
            info(f"  ✓ org '{slug}' already exists", indent=1)
            out.append(existing_by_slug[slug])
            continue
        r = http.post(f"{app_url}/v1/orgs", json={
            "slug": slug,
            "display_name": display_name,
        })
        if r.status_code in (200, 201):
            info(f"  + created org '{slug}' ({display_name})", indent=1)
            out.append(r.json())
        else:
            info(f"  ! org create failed: {r.status_code} {r.text}", indent=1)
            r.raise_for_status()

    # Also include the individual account.
    r = http.get(f"{app_url}/v1/orgs")
    r.raise_for_status()
    for o in r.json():
        if o["account_type"] == "individual":
            info(f"  ✓ individual account: {o['display_name']}", indent=1)
            out.append(o)
            break
    return out


# ─── Step 2: agents ────────────────────────────────────────────────

def ensure_agents(chakra: ChakraMCP, accounts: list[dict[str, Any]], plan: Plan) -> list[dict[str, Any]]:
    """Spread `plan.agents` agents across the given accounts."""
    info(f"Step 2: ensuring {plan.agents} agents across {len(accounts)} accounts…")
    out: list[dict[str, Any]] = []

    # Round-robin agents across accounts so each org has a few.
    existing = {a["slug"]: a for a in chakra.agents.list()}
    for i, (slug, display_name, description) in enumerate(AGENT_NAMES[:plan.agents]):
        account = accounts[i % len(accounts)]
        # Use slug-only as the key (account_slug/slug is the global identity).
        if slug in existing:
            info(f"  ✓ agent '{slug}' already exists", indent=1)
            out.append(existing[slug])
            continue
        try:
            # 65% network, 25% org, 10% private — gives the
            # discovery/network pages something to render without
            # leaking everything publicly.
            roll = random.random()
            visibility = "network" if roll < 0.65 else ("org" if roll < 0.90 else "private")
            agent = chakra.agents.create({
                "account_id": account["id"],
                "slug": slug,
                "display_name": display_name,
                "description": description,
                "visibility": visibility,
            })
            info(f"  + created '{slug}' in {account['slug']} (visibility={visibility})", indent=1)
            out.append(agent)
        except ChakraMCPError as e:
            if e.status == 409:
                info(f"  ✓ '{slug}' already exists (skip)", indent=1)
                continue
            raise
    return out


# ─── Step 3: capabilities ──────────────────────────────────────────

def ensure_capabilities(
    chakra: ChakraMCP, agents: list[dict[str, Any]], plan: Plan
) -> list[dict[str, Any]]:
    """Spread `plan.capabilities` capabilities across the given agents.

    First `plan.public_invoke_caps` get `public_invoke=true` with a
    100/month quota — that's the surface the public-tier reviews
    will eventually attach to.
    """
    info(f"Step 3: ensuring {plan.capabilities} capabilities ({plan.public_invoke_caps} public)…")
    out: list[dict[str, Any]] = []

    for i, (cap_name, description) in enumerate(CAPABILITY_NAMES[:plan.capabilities]):
        # Round-robin assign cap to agent.
        agent = agents[i % len(agents)]
        is_public = i < plan.public_invoke_caps
        body: dict[str, Any] = {
            "name": cap_name,
            "description": description,
            "input_schema": DEFAULT_INPUT_SCHEMA,
            "output_schema": DEFAULT_OUTPUT_SCHEMA,
            "visibility": "network",
        }
        if is_public:
            # Public-invokable capabilities require visibility=network
            # and a non-null monthly quota. 100/month is generous for a
            # demo but still demonstrates the cap.
            body["public_invoke"] = True
            body["public_monthly_quota_per_agent"] = 100

        try:
            cap = chakra.agents.capabilities.create(agent["id"], body)
            tag = "public_invoke" if is_public else "friendship-only"
            info(f"  + {agent['slug']}.{cap_name}  [{tag}]", indent=1)
            cap["_agent"] = agent
            out.append(cap)
        except ChakraMCPError as e:
            if e.status == 409:
                # Already there — list + find.
                existing = chakra.agents.capabilities.list(agent["id"])
                for c in existing:
                    if c["name"] == cap_name:
                        info(f"  ✓ {agent['slug']}.{cap_name} (already)", indent=1)
                        c["_agent"] = agent
                        out.append(c)
                        break
                continue
            raise
    return out


# ─── Step 4: friendships ───────────────────────────────────────────

def ensure_friendships(
    chakra: ChakraMCP, agents: list[dict[str, Any]], plan: Plan
) -> list[dict[str, Any]]:
    """Propose `plan.friendships` agent-to-agent friendships and bring
    most of them to `accepted` so we have a graph thick enough to grant
    + invoke through.
    """
    info(f"Step 4: proposing {plan.friendships} friendships…")
    out: list[dict[str, Any]] = []

    # Sample without replacement so we don't propose the same pair
    # twice within one run. Across runs the 409 catches duplicates.
    rng = random.Random(42)  # deterministic — easier to reason about
    pairs: set[tuple[str, str]] = set()
    attempts = 0
    while len(pairs) < plan.friendships and attempts < plan.friendships * 4:
        a, b = rng.sample(agents, 2)
        if a["account_id"] == b["account_id"]:
            # Same-account agents are auto-friends via the org policy
            # for org accounts; for individual accounts they don't need
            # an explicit friendship. Skip to avoid duplicates.
            attempts += 1
            continue
        key = tuple(sorted([a["id"], b["id"]]))
        if key in pairs:
            attempts += 1
            continue
        pairs.add(key)

        try:
            f = chakra.friendships.propose({
                "proposer_agent_id": a["id"],
                "target_agent_id": b["id"],
                "proposer_message": f"hey {b['display_name']}, want to collaborate?",
            })
        except ChakraMCPError as e:
            if e.status in (409,):
                attempts += 1
                continue
            raise

        # Resolve to accepted / rejected / proposed per the plan mix.
        roll = rng.random()
        accept_cut = plan.accepted_pct
        reject_cut = plan.accepted_pct + plan.rejected_pct

        try:
            if roll < accept_cut:
                f = chakra.friendships.accept(f["id"], message="lgtm 🤝")
            elif roll < reject_cut:
                f = chakra.friendships.reject(f["id"], message="not right now, ping later")
            # else: leave as 'proposed'
        except ChakraMCPError as e:
            # Auto-friendship may have already accepted the row from
            # the org-policy backfill; swallow.
            if e.status not in (409,):
                raise

        out.append(f)
        attempts += 1
    info(f"  → {len(out)} friendship rows", indent=1)
    return out


# ─── Step 5: grants ────────────────────────────────────────────────

def ensure_grants(
    chakra: ChakraMCP, agents: list[dict[str, Any]],
    capabilities: list[dict[str, Any]], friendships: list[dict[str, Any]], plan: Plan
) -> list[dict[str, Any]]:
    """Issue `plan.grants` grants over accepted friendships.

    A grant ties (granter_agent, grantee_agent, capability) — the
    capability must belong to the granter. Some grants get revoked at
    creation+ε to populate the audit log.
    """
    info(f"Step 5: issuing {plan.grants} grants ({plan.revoked_grants} will be revoked)…")
    out: list[dict[str, Any]] = []

    by_agent_caps: dict[str, list[dict[str, Any]]] = {}
    for c in capabilities:
        by_agent_caps.setdefault(c["_agent"]["id"], []).append(c)

    accepted = [f for f in friendships if f.get("status") == "accepted"]
    if not accepted:
        info("  ! no accepted friendships → no grants", indent=1)
        return out

    rng = random.Random(7)
    rng.shuffle(accepted)
    for i, f in enumerate(accepted[:plan.grants]):
        # Granter = the side that has at least one capability.
        granter, grantee = f["proposer"], f["target"]
        if granter["id"] not in by_agent_caps:
            granter, grantee = grantee, granter
            if granter["id"] not in by_agent_caps:
                continue
        cap = rng.choice(by_agent_caps[granter["id"]])
        try:
            g = chakra.grants.create({
                "granter_agent_id": granter["id"],
                "grantee_agent_id": grantee["id"],
                "capability_id": cap["id"],
            })
        except ChakraMCPError as e:
            if e.status == 409:
                continue
            raise
        g["_capability"] = cap
        g["_granter"] = granter
        g["_grantee"] = grantee
        info(f"  + {grantee['slug']} → {granter['slug']}.{cap['name']}", indent=1)

        # Revoke the last N to populate revoked audit rows.
        if i >= plan.grants - plan.revoked_grants:
            try:
                chakra.grants.revoke(g["id"], reason="demo: revoked to populate audit log")
                info(f"    ↳ revoked", indent=2)
                g["status"] = "revoked"
            except ChakraMCPError:
                pass
        out.append(g)
    return out


# ─── Step 6: invocations ───────────────────────────────────────────

def _pull_then_respond(
    http: httpx.Client, relay_url: str, granter_agent_id: str, invocation_id: str,
    *, status: str, output: Any = None, error: str | None = None
) -> None:
    """Two-step result post:

      1. GET /v1/inbox?agent_id=<granter>  — claims any pending rows
         for that agent, transitioning each from `pending` →
         `in_progress`. The relay rejects `/result` POSTs on a
         `pending` row with 409 "only in_progress can be completed".

      2. POST /v1/invocations/{id}/result with the body. By now the
         row is `in_progress` and the post is accepted.

    Both calls use the same Bearer key — the caller owns every agent
    in this single-user seeder, so authz passes trivially.
    """
    # Step 1: claim. We don't care about the response; the side-effect
    # (state transition) is what we want. Idempotent: re-pulling
    # in_progress rows doesn't error.
    http.get(f"{relay_url}/v1/inbox", params={"agent_id": granter_agent_id})

    # Step 2: actually post the result.
    body: dict[str, Any] = {"status": status}
    if output is not None:
        body["output"] = output
    if error is not None:
        body["error"] = error
    http.post(f"{relay_url}/v1/invocations/{invocation_id}/result", json=body)


def seed_invocations(
    http: httpx.Client, chakra: ChakraMCP, relay_url: str,
    grants: list[dict[str, Any]], plan: Plan
) -> list[dict[str, Any]]:
    """Run `plan.invocations` real round-trips through /v1/invoke +
    /v1/invocations/{id}/result.

    Status mix (rough):
      - 65% succeeded
      - 20% failed (granter responds status=failed)
      - 15% rejected (we POST /v1/invoke with a *revoked* grant —
        relay rejects pre-flight, creating a 'rejected' audit row
        without ever touching the granter)
    """
    active = [g for g in grants if g.get("status") != "revoked"]
    revoked = [g for g in grants if g.get("status") == "revoked"]
    if not active:
        info("  ! no active grants → no invocations", indent=1)
        return []

    info(f"Step 6: running {plan.invocations} invocations through the relay…")
    out: list[dict[str, Any]] = []
    rng = random.Random(11)

    # 15% rejected (need a revoked grant to produce one)
    n_rejected = int(plan.invocations * 0.15) if revoked else 0
    n_failed = int(plan.invocations * 0.20)
    n_succeeded = plan.invocations - n_rejected - n_failed

    for i in range(plan.invocations):
        if i < n_succeeded:
            target = "succeeded"; pool = active
        elif i < n_succeeded + n_failed:
            target = "failed";    pool = active
        else:
            target = "rejected";  pool = revoked or active

        g = rng.choice(pool)
        cap = g["_capability"]; granter = g["_granter"]; grantee = g["_grantee"]

        body = {
            "grant_id": g["id"],
            "grantee_agent_id": grantee["id"],
            "input": {"text": rng.choice([
                "ping", "summarise: the team launched a new feature today",
                "fri 3pm available?", "translate to fr: hello world",
                "what's the SLA on incident response?",
                "estimate completion: refactor of inbox.serve",
            ])},
        }
        try:
            enq = chakra.invoke(body)
        except ChakraMCPError as e:
            # If we picked a revoked grant we expect a 4xx + 'rejected'
            # row in the audit log — the relay logs it on the way out.
            if e.status in (400, 403, 404, 409):
                # The relay still wrote an invocation row with
                # status='rejected'. Count it as one of ours.
                out.append({"target_status": "rejected", "skipped_response": True})
                if (i + 1) % 25 == 0:
                    info(f"  • {i + 1}/{plan.invocations} invocations…", indent=1)
                continue
            raise

        if target in ("succeeded", "failed"):
            if target == "succeeded":
                _pull_then_respond(
                    http, relay_url, granter["id"], enq["invocation_id"],
                    status="succeeded",
                    output={"reply": f"OK — handled by {granter['display_name']}"},
                )
            else:
                _pull_then_respond(
                    http, relay_url, granter["id"], enq["invocation_id"],
                    status="failed",
                    error=rng.choice([
                        "upstream timeout",
                        "input failed validation: required field 'context' missing",
                        "downstream API returned 502",
                        "model returned malformed JSON",
                    ]),
                )

        out.append({
            "invocation_id": enq["invocation_id"],
            "target_status": target,
            "capability_name": cap["name"],
        })
        if (i + 1) % 25 == 0:
            info(f"  • {i + 1}/{plan.invocations} invocations…", indent=1)

    info(f"  → {len(out)} invocation rows created", indent=1)
    return out


# ─── Step 7: reviews ───────────────────────────────────────────────

def seed_reviews(
    chakra: ChakraMCP, grants: list[dict[str, Any]],
    plan: Plan
) -> list[dict[str, Any]]:
    """Write `plan.reviews` reviews. Each review is (grantee_agent →
    granter_agent), tagged with the capability the grantee invoked.

    Star distribution biased toward 4-5★ so the demo's average lands
    around 4.2-4.5 — believable for a thriving agent ecosystem.
    """
    info(f"Step 7: writing {plan.reviews} reviews…")
    out: list[dict[str, Any]] = []
    rng = random.Random(23)

    # Distribution: 5★=45%, 4★=30%, 3★=15%, 2★=7%, 1★=3%.
    star_weights = [(5, 45), (4, 30), (3, 15), (2, 7), (1, 3)]
    star_pool = [s for s, w in star_weights for _ in range(w)]

    # Each review needs a unique (reviewer, target) pair. Walk grants
    # in random order and write until we hit the target or exhaust.
    rng.shuffle(grants)
    seen_pairs: set[tuple[str, str]] = set()
    for g in grants:
        if len(out) >= plan.reviews:
            break
        reviewer = g["_grantee"]; target = g["_granter"]; cap = g["_capability"]
        key = (reviewer["id"], target["id"])
        if key in seen_pairs:
            continue
        seen_pairs.add(key)

        rating = rng.choice(star_pool)
        comment_pool = REVIEW_COMMENTS[rating]
        comment = rng.choice(comment_pool) or None
        try:
            r = chakra.reviews.write(target["id"], {
                "reviewer_agent_id": reviewer["id"],
                "rating": rating,
                "comment": comment,
                "tagged_capability_ids": [cap["id"]],
            })
            out.append(r)
            tag = "💬" if comment else "  "
            info(f"  {tag} {reviewer['slug']} → {target['slug']}: {rating}★", indent=1)
        except ChakraMCPError as e:
            # The most common failure here is "you haven't invoked this
            # capability" — happens for grants we created but never
            # actually invoked through. Skip & try the next pair.
            if e.status == 400:
                continue
            raise
    return out


# ─── Main ───────────────────────────────────────────────────────────

def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--scale", choices=list(PLANS), default="heavy",
                    help="How much data to create (default: heavy)")
    ap.add_argument("--app-url", default=os.environ.get("CHAKRAMCP_APP_URL", "https://app.chakramcp.com"))
    ap.add_argument("--relay-url", default=os.environ.get("CHAKRAMCP_RELAY_URL", "https://relay.chakramcp.com"))
    args = ap.parse_args()

    api_key = os.environ.get("CHAKRAMCP_API_KEY")
    if not api_key:
        print("set CHAKRAMCP_API_KEY first (get one from chakramcp.com → Settings → API keys)", file=sys.stderr)
        sys.exit(2)

    plan = PLANS[args.scale]
    info(f"=== ChakraMCP demo seeder — scale=`{args.scale}` ===")
    info(f"   app:   {args.app_url}")
    info(f"   relay: {args.relay_url}")
    info("")

    headers = {
        "authorization": f"Bearer {api_key}",
        "user-agent": "chakramcp-demo-seeder/1.0",
    }
    http = httpx.Client(headers=headers, timeout=30.0)
    chakra = ChakraMCP(api_key=api_key, app_url=args.app_url, relay_url=args.relay_url,
                       http_client=httpx.Client(headers=headers, timeout=30.0))

    started = time.monotonic()
    seeded = Seeded()

    seeded.orgs          = ensure_orgs(http, args.app_url, plan)
    seeded.agents        = ensure_agents(chakra, seeded.orgs, plan)
    seeded.capabilities  = ensure_capabilities(chakra, seeded.agents, plan)
    seeded.friendships   = ensure_friendships(chakra, seeded.agents, plan)
    seeded.grants        = ensure_grants(chakra, seeded.agents, seeded.capabilities, seeded.friendships, plan)
    seeded.invocations   = seed_invocations(http, chakra, args.relay_url, seeded.grants, plan)
    seeded.reviews       = seed_reviews(chakra, seeded.grants, plan)

    elapsed = time.monotonic() - started
    info("")
    info("=== DONE ===")
    info(f"   orgs:         {len(seeded.orgs)}")
    info(f"   agents:       {len(seeded.agents)}")
    info(f"   capabilities: {len(seeded.capabilities)}")
    info(f"   friendships:  {len(seeded.friendships)}")
    info(f"   grants:       {len(seeded.grants)}")
    info(f"   invocations:  {len(seeded.invocations)}")
    info(f"   reviews:      {len(seeded.reviews)}")
    info(f"   elapsed:      {elapsed:.1f}s")


if __name__ == "__main__":
    main()
