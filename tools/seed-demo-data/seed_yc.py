#!/usr/bin/env python3
"""
YC-themed demo seeder for ChakraMCP.

Replaces generic seed data with a realistic three-org YC investment
workflow:

  yc-applications  — intake + vet applications (deck summary, market
                     sizing, founder vetting, financial triage)
  yc-interviews    — schedule + run technical and founder interviews
  yc-evaluation    — partner synthesis, rubric scoring, decision memos

Application threads flow through the system:

  intake  →  deck summary + market + team + financials
           →  interview prep → tech + founder interviews
           →  partner synthesis → rubric → decision memo

Each invocation carries a real-looking payload tied to a specific
application (Volta Health, Vexa Robotics, etc.). Reviews read like
partner-circle feedback.

Phase 0 tears down the existing dummy structure (delete all agents
owned by the caller's accounts + drop the dummy orgs `acme-labs` /
`trip-co`).

Phase 8 emits `backdate.sql` — a one-shot patch you apply server-side
with `psql -f backdate.sql` to spread the freshly-created
relay_invocations + agent_reviews rows across the last 14 days.

Usage:
    pip install chakramcp-sdk httpx
    CHAKRAMCP_API_KEY=ck_... python3 seed_yc.py
"""
from __future__ import annotations

import argparse
import os
import random
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

import httpx

try:
    from chakramcp import ChakraMCP, ChakraMCPError
except ImportError:
    print("Please `pip install chakramcp-sdk httpx` first.", file=sys.stderr)
    sys.exit(1)


# ─── Org & agent layout ────────────────────────────────────────────

ORG_SPECS = [
    ("yc-applications", "YC Applications"),
    ("yc-interviews",   "YC Interview Coordinator"),
    ("yc-evaluation",   "YC Evaluation Panel"),
]

# (slug, display_name, org_slug, description, visibility, capabilities)
# Each capability: (name, description, public_invoke)
AGENT_SPECS = [
    # —— APPLICATIONS org ——
    ("deck-summarizer", "Deck Summarizer", "yc-applications",
     "Reads pitch decks + extracts the 200-word digest partners actually need.",
     "org",
     [("summarize_deck", "Read a pitch deck PDF, return key sections", True),
      ("extract_traction", "Pull MRR/MAU/churn/etc out of an application", False)]),
    ("founder-vetter", "Founder Vetter", "yc-applications",
     "LinkedIn + GitHub + paper-trail check on founder backgrounds.",
     "org",
     [("check_founding_team", "Vet the founding team — backgrounds, prior exits, references", True),
      ("reference_check", "Email + Slack outreach to listed references", False)]),
    ("market-analyst", "Market Analyst", "yc-applications",
     "TAM/SAM/SOM sizing + competitive landscape.",
     "org",
     [("size_market", "Estimate TAM, SAM, SOM with sources", True),
      ("competitor_scan", "Identify direct + adjacent competitors", False)]),
    ("financial-triage", "Financial Triage", "yc-applications",
     "Burn, runway, unit economics from the application doc.",
     "private",
     [("triage_financials", "Sanity-check the stated burn / runway / ask", False),
      ("score_unit_economics", "Compute LTV/CAC, payback, contribution margin", False)]),

    # —— INTERVIEWS org ——
    ("interview-scheduler", "Interview Scheduler", "yc-interviews",
     "Books slots across partners + founders, generates prep briefs.",
     "org",
     [("schedule_interview", "Find a slot across partner + founder calendars", True),
      ("prep_partner_brief", "1-page brief sent to partner before the call", False)]),
    ("tech-interviewer", "Technical Interviewer", "yc-interviews",
     "Runs the 30-min technical depth interview — system design, prior code.",
     "org",
     [("conduct_technical_round", "Run the technical interview, return transcript + signals", False),
      ("score_tech_depth", "Rate technical depth across system design + execution", False)]),
    ("founder-interviewer", "Founder Story Interviewer", "yc-interviews",
     "The 'why you, why now' interview — motivation, durability, conflict.",
     "org",
     [("conduct_founder_story", "Run the founder-story interview, return transcript", False),
      ("probe_motivation", "Targeted follow-ups on why-now and conflict resolution", False)]),

    # —— EVALUATION org ——
    ("partner-synthesis", "Partner Synthesis", "yc-evaluation",
     "Synthesizes inputs from applications + interviews; routes back to either for more info.",
     "network",
     [("synthesize_partner_notes", "Merge application + interview signal into a partner-ready brief", False),
      ("request_more_info", "Send a targeted question back to applications or interviews", False)]),
    ("rubric-scorer", "Rubric Scorer", "yc-evaluation",
     "Standardised 0-10 scoring across team, market, traction, ask.",
     "network",
     [("compute_rubric_score", "Score against the YC rubric, with per-axis justification", False),
      ("compare_to_cohort", "Position this application against the live cohort percentiles", False)]),
    ("decision-memo", "Decision Memo Writer", "yc-evaluation",
     "Drafts the partner-vote-ready memo: thesis, risks, asks.",
     "network",
     [("draft_decision_memo", "Compose the final partner-vote memo", False),
      ("flag_concerns", "Surface the top 3 risks for partner discussion", False)]),
    ("cohort-curator", "Cohort Curator", "yc-evaluation",
     "Optimises the batch for diversity of sector, stage, geography.",
     "network",
     [("curate_cohort", "Suggest accept/reject deltas to balance the cohort", False),
      ("score_strategic_fit", "Score how well this application complements existing accepts", False)]),
]


# ─── Application thread templates ──────────────────────────────────

@dataclass
class Application:
    id: str
    company: str
    founders: str
    sector: str
    stage: str
    ask: str
    one_liner: str
    deck_summary: str = ""
    traction: str = ""
    market: str = ""
    team_score: str = ""
    financials_note: str = ""
    tech_signal: str = ""
    founder_signal: str = ""
    rubric: str = ""
    decision: str = ""
    flag: str = ""


APPLICATIONS: list[Application] = [
    Application("APP-2026-0431", "Volta Health",
                "Maya Chen (ex-Calm PM, 4y), Diego Reyes (Stanford CS PhD, prev Stripe ML)",
                "B2B health-data infra", "pre-seed", "$1.5M @ $12M post",
                "FHIR-native data pipeline for primary-care groups — wraps Epic + Athena into one API."),
    Application("APP-2026-0432", "Vexa Robotics",
                "Priya Kapoor (ex-Amazon Robotics lead, 6y), Tom Liu (CMU MechE)",
                "warehouse automation", "seed", "$3M @ $22M post",
                "Modular pick-arm rentable per-month, replaces a $90k commit with a $1.8k/mo OPEX line."),
    Application("APP-2026-0433", "Tessera Studios",
                "Aoife Murphy (TCD CS, ex-Riot), Sven Adler (game industry vet, 12y)",
                "indie game tooling", "pre-seed", "$800k @ $7M post",
                "Multiplayer netcode-as-a-service for solo developers; pricing is per-CCU."),
    Application("APP-2026-0434", "Mosaic Tax",
                "Anika Sharma (ex-Big4 tax partner), Wei Zhang (TurboTax SDE 8y)",
                "small-business tax SaaS", "seed", "$2M @ $14M post",
                "Bookkeeping → tax-return autopilot for SMBs under $5M revenue."),
    Application("APP-2026-0435", "Foundry Climate",
                "Rajesh Iyer (ex-Tesla supply chain), Kara O'Brien (MIT EE PhD)",
                "carbon-MRV", "seed", "$2.5M @ $18M post",
                "Embedded carbon-accounting in industrial PLCs; sells through SCADA partners."),
    Application("APP-2026-0436", "Lumen AI",
                "Felix Tan (Cohere alum, 3y), Riya Pandey (DeepMind RE, 2y)",
                "developer-tool AI", "pre-seed", "$1.2M @ $10M post",
                "Codegen for SQL — dialect-aware, schema-aware, audit-logged."),
    Application("APP-2026-0437", "Aster Fertility",
                "Dr. Lena Park (board-certified REI, 10y clinical), Jamie Cole (Flatiron SWE)",
                "fertility tooling", "seed", "$3.5M @ $25M post",
                "EMR + patient comms purpose-built for fertility clinics; replaces FertilityProAccess."),
    Application("APP-2026-0438", "Helix Logistics",
                "Marcus Patel (ex-Convoy ops), Eun-ji Kang (ex-Flexport eng)",
                "freight forwarding", "seed", "$2.2M @ $17M post",
                "LCL ocean booking + customs for SMB exporters; 3 lanes live (LAX↔PVG, JFK↔HAM, MIA↔SCL)."),
    Application("APP-2026-0439", "Ceres Agrotech",
                "Bayo Adeyemi (Lagos, ex-OneAcre ops), Chris Olsson (ETH Zurich agronomy)",
                "smallholder lending", "pre-seed", "$1M @ $9M post",
                "Yield-indexed crop loans for smallholders in Nigeria + Kenya; underwriting via satellite imagery."),
    Application("APP-2026-0440", "Beacon Security",
                "Ravi Iyengar (ex-Google Project Zero), Sarah Birch (UCB PhD crypto)",
                "developer security", "seed", "$3M @ $24M post",
                "Pre-commit dependency vulnerability auto-fix; ships as a GitHub App."),
    Application("APP-2026-0441", "Hearth Education",
                "Adeline Park (ex-Khan Academy curriculum), Yusuf Demir (ex-Replit eng)",
                "K-12 personalised learning", "pre-seed", "$1.4M @ $11M post",
                "After-school tutor bot calibrated against the Common Core item bank."),
    Application("APP-2026-0442", "Helia Battery",
                "Dr. Anand Gupta (MIT Lincoln Labs vet), Lily Chen (Tesla 1st-gen Powerwall)",
                "stationary storage", "seed", "$4M @ $30M post",
                "Sodium-ion battery cells for non-mobile applications; 3 utility pilots signed."),
    Application("APP-2026-0443", "Salient Reps",
                "Connor Walsh (ex-Outreach AE), Maria Hernandez (Twilio voice eng)",
                "sales-AI", "pre-seed", "$1.3M @ $9.5M post",
                "AI SDR that books meetings from inbound forms; positioning vs Apollo Reach."),
    Application("APP-2026-0444", "Quanta Diagnostics",
                "Dr. Hina Bhatti (clinical pathologist), Eli Reisman (DeepMind imaging RE)",
                "pathology AI", "seed", "$3.2M @ $26M post",
                "Whole-slide image triage for FNAC cytology; CE-marked, US FDA submission in Q3."),
    Application("APP-2026-0445", "Trellis Govtech",
                "Naomi Stein (ex-USDS), Brad Levin (ex-Palantir FDE)",
                "civic procurement", "pre-seed", "$1M @ $8M post",
                "Plain-language RFP search for SMBs bidding on US state contracts."),
    Application("APP-2026-0446", "Stellar Brew",
                "Sam Okonkwo (specialty coffee veteran), Jade Park (DTC ops, ex-Hims)",
                "DTC consumer", "seed", "$2.5M @ $18M post",
                "Single-origin subscription with a flavor-graph recommender — claims 38% retention at M6."),
    Application("APP-2026-0447", "Auric Insurance",
                "Tasha Williams (ex-Lemonade actuarial), David Patel (ex-MetroMile data)",
                "embedded insurance", "seed", "$3M @ $24M post",
                "API-first GL insurance for gig-economy fleets; 2 marketplaces piloting."),
    Application("APP-2026-0448", "Mira Robotics",
                "Hiroshi Tanaka (ex-Toyota Research), Naomi Lee (Stanford SAIL)",
                "humanoid micro-tasks", "pre-seed", "$2M @ $15M post",
                "Bench-top humanoid forks for lab automation; first customer is a UCSF wet lab."),
    Application("APP-2026-0449", "Polaris Drones",
                "Ethan Brooks (ex-Skydio ML), Aanya Roy (ex-Joby autopilot)",
                "aerial inspection", "seed", "$3.5M @ $28M post",
                "Insurance-grade roof inspection drones; replaces $400 hand-inspection with a $35 drone pass."),
    Application("APP-2026-0450", "Synapse Labs",
                "Dr. Karthik Iyer (CMU BMI), Anna Nilsson (KTH neural-eng)",
                "BCI dev tools", "pre-seed", "$1.5M @ $13M post",
                "Higher-level SDK on top of OpenBCI hardware; lowers time-to-first-prototype from weeks to a day."),
    Application("APP-2026-0451", "Hub Dental",
                "Dr. Selma Akande (DDS, 8y practice), Robert Yates (ex-Athena healthtech)",
                "dental practice ops", "seed", "$2M @ $15M post",
                "EMR + recall agent for small dental practices; net-new market vs Dentrix."),
    Application("APP-2026-0452", "Glacier Search",
                "Olivia Park (ex-Pinterest ML), Sho Tanaka (ex-Algolia infra)",
                "infra B2B", "pre-seed", "$1.2M @ $10M post",
                "Semantic search for cold-storage data lakes; targets the 'we don't query that anymore' tier."),
    Application("APP-2026-0453", "Cascade Compliance",
                "Megha Reddy (Big4 compliance, 6y), Jordan Hewitt (ex-Vanta eng)",
                "SOC2 / ISO automation", "seed", "$2.5M @ $20M post",
                "Continuous-controls for sub-Series-B SaaS; positioning vs Drata + Vanta."),
    Application("APP-2026-0454", "Folio Studios",
                "Aurora Diaz (ex-Notion design lead), Kenji Sato (designer-tool eng)",
                "creator tooling", "pre-seed", "$1M @ $8.5M post",
                "Portfolio-as-a-platform for designers; built-in shop, payments, analytics."),
    Application("APP-2026-0455", "Bayou Power",
                "Lucia Vasquez (ex-Sunrun ops), Daniel Hsu (Tesla powerwall installer-eng)",
                "DER VPP", "seed", "$3M @ $24M post",
                "Aggregates residential batteries into a VPP; 2 ISOs piloting in TX + CA."),
    Application("APP-2026-0456", "Nimbus Audio",
                "Pia Saito (ex-Discord audio eng), Tarek Ahmed (CMU CS)",
                "voice AI infra", "pre-seed", "$1.5M @ $12M post",
                "Sub-100ms voice AI relay for game studios; competes with LiveKit voice."),
    Application("APP-2026-0457", "Loom Hardware",
                "Jonas Weber (ex-Apple silicon), Mei Lin (ex-NVIDIA compiler)",
                "compiler infra", "seed", "$4M @ $32M post",
                "Domain-specific accelerators-as-a-library; targets ML inference for sub-7B models."),
    Application("APP-2026-0458", "Pioneer Care",
                "Dr. Naomi Akwa (geriatrician), Caleb Foster (ex-Honor eng)",
                "in-home care ops", "seed", "$2.5M @ $18M post",
                "Scheduling + claims automation for in-home care agencies; ~$200B market."),
    Application("APP-2026-0459", "Cinder Robotics",
                "Petra Ostrowski (Boston Dynamics alum), Ravi Patel (ex-Plus.ai)",
                "industrial inspection", "pre-seed", "$2M @ $16M post",
                "Quadruped robots for refinery inspection; first contract with Chevron Pasadena."),
    Application("APP-2026-0460", "Whetstone Talent",
                "Yui Tanaka (ex-Greenhouse PM), Lewis Brown (ex-Lever eng)",
                "recruiting ops", "pre-seed", "$1.2M @ $9M post",
                "Calibration-driven ATS for series-A startups; replaces Greenhouse for sub-50-person co's."),
]

# Pre-populate the per-application content used as invocation
# input/output payloads. Keeps the chain code clean.
def hydrate(app: Application) -> None:
    app.deck_summary = (
        f"{app.company} ({app.id}): {app.one_liner} "
        f"Founders: {app.founders}. Sector: {app.sector}. "
        f"Stage: {app.stage}. Ask: {app.ask}."
    )
    rng = random.Random(hash(app.id))
    app.traction = rng.choice([
        f"$8k MRR, 22% MoM growth, 14 paying logos (avg ${rng.randint(120, 600)}/mo).",
        f"3 LOIs ($1.4M ACV) + 1 signed contract ($420k ACV) with a single 4-mo POC.",
        f"Pre-revenue. 8 design partners; first paid in ~6 wks.",
        f"$32k MRR, 38% MoM, 91 paying logos. 4% logo churn at M6.",
        f"$110k MRR, 9% MoM, 18 enterprise pilots converting at ~$28k ACV each.",
    ])
    app.market = rng.choice([
        f"TAM ${rng.randint(2,20)}B (2026 IDC); SAM ${rng.randint(400,2400)}M; SOM ${rng.randint(40,160)}M at 3% share.",
        f"TAM ${rng.randint(8,40)}B (Gartner 2025); SAM ${rng.randint(900,3600)}M (US mid-market).",
        f"TAM ${rng.randint(1,5)}B (bottoms-up); 12k addressable buyers × est ${rng.randint(15,90)}k ACV.",
    ])
    team_axes = [
        ("technical depth", rng.randint(6, 10)),
        ("domain experience", rng.randint(5, 10)),
        ("prior founding", rng.randint(2, 8)),
        ("reference signal", rng.randint(5, 10)),
    ]
    app.team_score = "; ".join(f"{axis} {score}/10" for axis, score in team_axes)
    app.financials_note = rng.choice([
        f"Stated burn ${rng.randint(35, 220)}k/mo; runway looks ~{rng.randint(14, 28)} mo at ask. CAC clean.",
        f"Burn ${rng.randint(50, 300)}k/mo; LTV/CAC = {rng.choice(['1.8x', '2.6x', '4.1x', '5.8x'])} (early payback flag).",
        f"Pre-revenue. Implied 18-mo runway. Founder salaries below market — clean.",
    ])
    app.tech_signal = rng.choice([
        f"Strong on system design — walked the {rng.choice(['streaming dedupe', 'multi-tenant isolation', 'p99 latency'])} question end-to-end. Code-quality signal: high.",
        f"Decent. Some hand-waving on {rng.choice(['scalability past 10k QPS', 'multi-region failover', 'auth model'])} — flagged for follow-up.",
        f"Excellent. Top-decile of the cohort on the {rng.choice(['rate-limit', 'idempotency', 'schema-evolution'])} drill-down.",
    ])
    app.founder_signal = rng.choice([
        "Convincing on 'why now' — points to the regulatory shift in Q2. Founder market fit: strong.",
        "Story holds together; weak on 'why us'. Probed prior conflicts — recovered well.",
        "Magnetic. Articulated the 10-yr vision without overselling. Likely fundable independent of YC.",
        "Articulate but cautious. Pushed on motivation; founder admitted hedging vs a senior role at FAANG.",
    ])
    rubric_total = rng.uniform(5.8, 9.4)
    app.rubric = (
        f"Team: {rng.uniform(5,10):.1f}/10  |  "
        f"Market: {rng.uniform(5,10):.1f}/10  |  "
        f"Traction: {rng.uniform(4,10):.1f}/10  |  "
        f"Ask: {rng.uniform(5,10):.1f}/10  |  "
        f"OVERALL: {rubric_total:.1f}/10"
    )
    if rubric_total >= 7.8:
        app.decision = "ADVANCE to partner-circle vote. Strong-buy notes from 2/3 partners."
        app.flag = "Watch: dilution math at the next round given current ask."
    elif rubric_total >= 6.6:
        app.decision = "BORDERLINE. Schedule a follow-up call with the founders on the founder-market-fit thread."
        app.flag = f"Concerns: {rng.choice(['founder-market fit narrowness', 'thin reference signal', 'ask is rich for the stage', 'unclear moat at scale'])}."
    else:
        app.decision = "PASS for this cycle. Encouraging note recommending the team re-apply at the next batch."
        app.flag = f"Top reasons: {rng.choice(['team gaps', 'market size below threshold', 'unclear path to gross-margin', 'unconvincing why-now'])}."


for app in APPLICATIONS:
    hydrate(app)


# ─── Partner review corpus ─────────────────────────────────────────

# Reviews are written by evaluation-org agents about applications-org
# and interviews-org agents. They read like partner-circle feedback.
REVIEW_CORPUS: dict[int, list[str]] = {
    5: [
        "Saved me 40 minutes per applicant. Deck digests are tight and unbiased — partner-ready.",
        "Best signal-to-noise on the whole pipeline. Lands the founder-market-fit takeaway in one sentence.",
        "Reference checks have been spot-on twice this cycle — including the one where a founder forgot to disclose a co-founder split.",
        "Tech-interview transcripts are the right level of detail — readable in 3 minutes, decisive.",
        "Cohort-percentile comparison is what unlocked the partner-vote on Tessera Studios. Keep this.",
        "",
    ],
    4: [
        "Strong overall. Market sizes occasionally pull from a 2023 source — would like the model card to say which DB.",
        "Solid. Sometimes the founder-story output reads more like a CRM note than a partner brief.",
        "Useful 8/10 times. Once flagged the wrong concern as 'top risk' — I'd want a second-pass.",
        "Reliable for the standard track. Less useful when an application is anomalous (foreign founders, atypical stage).",
        "",
    ],
    3: [
        "Hit rate is OK. Half the time the rubric scores feel like the model is averaging to safety rather than committing.",
        "Useful as a first cut. I still re-read the deck before voting — defeats the purpose for me.",
        "Output schema changed twice this week. Synced with the team to lock it.",
        "Tech transcripts are getting longer over time — pushing past 1500 words is too much for partners to skim.",
        "",
    ],
    2: [
        "Founder-story output last week was bordering on hagiography for one of the founders. Tone needs work.",
        "Got the burn math wrong on Aster — said 22mo runway when the right number is 14. We caught it; that's a coin-flip.",
        "Misclassified Helia as 'pre-seed' when they're clearly seed. Stage detection is brittle.",
        "Drafted a memo that included a competitor that doesn't exist (Vanguard Bio). Hallucination is a deal-killer here.",
        "",
    ],
    1: [
        "Wrong on TAM for Foundry by 3x — pulled an old IBM estimate. Almost cost us a meeting.",
        "Reference check 'cleared' a founder whose previous co-founder formally disputed the cap table. Unacceptable miss.",
        "Memo template kept re-emitting the boilerplate intro for three different companies. Re-prompt or revoke.",
        "Decision memo recommended ADVANCE on a company with 1.8x LTV/CAC and 4mo runway. Hard pass on the output and on the company.",
        "",
    ],
}


# ─── Helpers ───────────────────────────────────────────────────────

def info(msg: str, *, indent: int = 0) -> None:
    print(f"{'  ' * indent}{msg}", flush=True)


@dataclass
class Seeded:
    orgs: list[dict[str, Any]] = field(default_factory=list)
    agents: dict[str, dict[str, Any]] = field(default_factory=dict)
    caps: dict[tuple[str, str], dict[str, Any]] = field(default_factory=dict)  # (agent_slug, cap_name) → cap
    friendships: list[dict[str, Any]] = field(default_factory=list)
    grants: dict[tuple[str, str, str], dict[str, Any]] = field(default_factory=dict)  # (granter_slug, grantee_slug, cap_name) → grant
    invocations: list[dict[str, Any]] = field(default_factory=list)
    reviews: list[dict[str, Any]] = field(default_factory=list)


# ─── Phase 0: teardown ─────────────────────────────────────────────

def teardown(http: httpx.Client, chakra: ChakraMCP, app_url: str) -> None:
    info("Phase 0: tearing down existing dummy data…")

    # 1. Delete every agent the user can see + admin.
    agents = chakra.agents.list()
    for a in agents:
        try:
            chakra.agents.delete(a["id"])
            info(f"  - deleted agent {a['account_slug']}/{a['slug']}", indent=1)
        except ChakraMCPError as e:
            info(f"  ! couldn't delete {a['slug']}: {e.status} {e.message}", indent=1)

    # 2. Drop the legacy demo orgs. The relay's DELETE /v1/orgs/{slug}
    #    cascades to agents (already gone) + memberships.
    r = http.get(f"{app_url}/v1/orgs")
    r.raise_for_status()
    legacy = [o for o in r.json() if o["slug"] in {"acme-labs", "trip-co"}]
    for o in legacy:
        rd = http.delete(f"{app_url}/v1/orgs/{o['slug']}")
        if rd.status_code < 300:
            info(f"  - dropped org '{o['slug']}'", indent=1)
        else:
            info(f"  ! couldn't drop '{o['slug']}': {rd.status_code} {rd.text[:120]}", indent=1)


# ─── Phase 1: orgs ─────────────────────────────────────────────────

def ensure_orgs(http: httpx.Client, app_url: str) -> dict[str, dict[str, Any]]:
    info("Phase 1: ensuring 3 YC orgs…")
    out: dict[str, dict[str, Any]] = {}

    r = http.get(f"{app_url}/v1/orgs")
    r.raise_for_status()
    existing = {o["slug"]: o for o in r.json()}

    for slug, display_name in ORG_SPECS:
        if slug in existing:
            info(f"  ✓ '{slug}' already there", indent=1)
            out[slug] = existing[slug]
            continue
        rc = http.post(f"{app_url}/v1/orgs", json={"slug": slug, "display_name": display_name})
        if rc.status_code in (200, 201):
            out[slug] = rc.json()
            info(f"  + created '{slug}' ({display_name})", indent=1)
        else:
            info(f"  ! create failed: {rc.status_code} {rc.text[:120]}", indent=1)
            rc.raise_for_status()

    # Pull the individual account too — anchor for any agents we want
    # under the user (none in this YC plan, but the next operator may).
    r = http.get(f"{app_url}/v1/orgs")
    r.raise_for_status()
    for o in r.json():
        if o["account_type"] == "individual":
            out["__individual__"] = o
            break
    return out


# ─── Phase 2: agents ───────────────────────────────────────────────

def ensure_agents(chakra: ChakraMCP, orgs: dict[str, dict[str, Any]]) -> dict[str, dict[str, Any]]:
    info("Phase 2: ensuring agents per org…")
    out: dict[str, dict[str, Any]] = {}

    existing = {a["slug"]: a for a in chakra.agents.list()}
    for slug, display, org_slug, desc, vis, _caps in AGENT_SPECS:
        if slug in existing:
            info(f"  ✓ {slug} already exists in {existing[slug]['account_slug']}", indent=1)
            out[slug] = existing[slug]
            continue
        org = orgs[org_slug]
        agent = chakra.agents.create({
            "account_id": org["account_id"] if "account_id" in org else org["id"],
            "slug": slug,
            "display_name": display,
            "description": desc,
            "visibility": vis,
        })
        out[slug] = agent
        info(f"  + {slug} → {org_slug} (visibility={vis})", indent=1)
    return out


# ─── Phase 3: capabilities ─────────────────────────────────────────

def ensure_caps(
    chakra: ChakraMCP, agents: dict[str, dict[str, Any]]
) -> dict[tuple[str, str], dict[str, Any]]:
    info("Phase 3: ensuring capabilities…")
    out: dict[tuple[str, str], dict[str, Any]] = {}

    input_schema = {
        "type": "object",
        "properties": {
            "application_id": {"type": "string"},
            "context": {"type": "string"},
        },
        "required": ["application_id"],
    }
    output_schema = {
        "type": "object",
        "properties": {
            "application_id": {"type": "string"},
            "result": {"type": "string"},
        },
    }

    for agent_slug, _disp, _org, _desc, _vis, caps in AGENT_SPECS:
        agent = agents[agent_slug]
        existing = {c["name"]: c for c in chakra.agents.capabilities.list(agent["id"])}
        for cap_name, cap_desc, public_invoke in caps:
            if cap_name in existing:
                out[(agent_slug, cap_name)] = existing[cap_name]
                continue
            body: dict[str, Any] = {
                "name": cap_name,
                "description": cap_desc,
                "input_schema": input_schema,
                "output_schema": output_schema,
                "visibility": "network" if public_invoke else "private",
            }
            if public_invoke:
                body["public_invoke"] = True
                body["public_monthly_quota_per_agent"] = 200
            try:
                c = chakra.agents.capabilities.create(agent["id"], body)
                out[(agent_slug, cap_name)] = c
                tag = "[public]" if public_invoke else "[org]"
                info(f"  + {agent_slug}.{cap_name} {tag}", indent=1)
            except ChakraMCPError as e:
                if e.status == 409:
                    existing = {c["name"]: c for c in chakra.agents.capabilities.list(agent["id"])}
                    out[(agent_slug, cap_name)] = existing[cap_name]
                else:
                    raise
    return out


# ─── Phase 4: friendships ──────────────────────────────────────────

# Cross-org friendships needed for the funnel:
#   evaluation ↔ applications  (eval queries app reviewers)
#   evaluation ↔ interviews    (eval queries interviewers)
#   interviews ↔ applications  (interviewer pulls app context)
FRIENDSHIP_PAIRS = [
    # Evaluation → Applications
    ("partner-synthesis",  "deck-summarizer"),
    ("partner-synthesis",  "founder-vetter"),
    ("partner-synthesis",  "market-analyst"),
    ("partner-synthesis",  "financial-triage"),
    ("rubric-scorer",      "deck-summarizer"),
    ("rubric-scorer",      "market-analyst"),
    ("rubric-scorer",      "financial-triage"),
    ("rubric-scorer",      "founder-vetter"),
    ("decision-memo",      "deck-summarizer"),
    ("decision-memo",      "founder-vetter"),
    ("cohort-curator",     "market-analyst"),
    ("cohort-curator",     "founder-vetter"),
    # Evaluation → Interviews
    ("partner-synthesis",  "tech-interviewer"),
    ("partner-synthesis",  "founder-interviewer"),
    ("partner-synthesis",  "interview-scheduler"),
    ("rubric-scorer",      "tech-interviewer"),
    ("rubric-scorer",      "founder-interviewer"),
    ("decision-memo",      "tech-interviewer"),
    ("decision-memo",      "founder-interviewer"),
    # Interviews → Applications (interviewers pull deck/team context)
    ("interview-scheduler","deck-summarizer"),
    ("tech-interviewer",   "deck-summarizer"),
    ("tech-interviewer",   "founder-vetter"),
    ("founder-interviewer","deck-summarizer"),
    ("founder-interviewer","founder-vetter"),
]


def ensure_friendships(
    chakra: ChakraMCP, agents: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    info("Phase 4: friendships…")
    out: list[dict[str, Any]] = []
    for a_slug, b_slug in FRIENDSHIP_PAIRS:
        a, b = agents[a_slug], agents[b_slug]
        try:
            f = chakra.friendships.propose({
                "proposer_agent_id": a["id"],
                "target_agent_id": b["id"],
                "proposer_message": f"opening a channel — need to call {b_slug}.* during eval",
            })
            f = chakra.friendships.accept(f["id"], message="ack 🤝")
            out.append(f)
        except ChakraMCPError as e:
            if e.status == 409:
                continue
            raise
    info(f"  → {len(out)} new accepted friendships", indent=1)
    return out


# ─── Phase 5: grants ───────────────────────────────────────────────

# Every friendship gets a grant on every capability the granter has.
def ensure_grants(
    chakra: ChakraMCP, agents: dict[str, dict[str, Any]],
    caps: dict[tuple[str, str], dict[str, Any]]
) -> dict[tuple[str, str, str], dict[str, Any]]:
    info("Phase 5: grants…")
    out: dict[tuple[str, str, str], dict[str, Any]] = {}

    # Build agent → its capabilities map.
    agent_caps: dict[str, list[tuple[str, dict[str, Any]]]] = {}
    for (agent_slug, cap_name), c in caps.items():
        agent_caps.setdefault(agent_slug, []).append((cap_name, c))

    # The friendship list above is "evaluator → applications/interviews"
    # form: the eval agent calls the app/interview agent's capability.
    # So the granter is the second slug (b), grantee is the first (a).
    for a_slug, b_slug in FRIENDSHIP_PAIRS:
        for cap_name, cap in agent_caps.get(b_slug, []):
            granter = agents[b_slug]; grantee = agents[a_slug]
            try:
                g = chakra.grants.create({
                    "granter_agent_id": granter["id"],
                    "grantee_agent_id": grantee["id"],
                    "capability_id": cap["id"],
                })
                out[(b_slug, a_slug, cap_name)] = g
            except ChakraMCPError as e:
                if e.status == 409:
                    continue
                raise
    info(f"  → {len(out)} grants issued", indent=1)
    return out


# ─── Phase 6: invocation chains ────────────────────────────────────

# (caller_agent, granter_agent, capability_name)
# Each application threads through all of these (in order). Each entry
# is a real round-trip.
APPLICATION_CHAIN = [
    # Intake fan-out
    ("partner-synthesis", "deck-summarizer",  "summarize_deck"),
    ("partner-synthesis", "market-analyst",   "size_market"),
    ("partner-synthesis", "founder-vetter",   "check_founding_team"),
    ("rubric-scorer",     "financial-triage", "triage_financials"),
    # Interview prep + execution
    ("partner-synthesis", "interview-scheduler", "schedule_interview"),
    ("partner-synthesis", "tech-interviewer",     "conduct_technical_round"),
    ("partner-synthesis", "founder-interviewer",  "conduct_founder_story"),
    # Synthesis → scoring → memo
    ("rubric-scorer",  "tech-interviewer",     "score_tech_depth"),
    ("rubric-scorer",  "founder-interviewer",  "probe_motivation"),
    ("decision-memo",  "founder-vetter",       "reference_check"),
    ("decision-memo",  "deck-summarizer",      "extract_traction"),
]

# Output renderers — given an application, produce a believable
# capability output payload.
def output_for(cap_name: str, app: Application) -> dict[str, Any]:
    if cap_name == "summarize_deck":
        return {"application_id": app.id, "summary": app.deck_summary}
    if cap_name == "size_market":
        return {"application_id": app.id, "sizing": app.market}
    if cap_name == "check_founding_team":
        return {"application_id": app.id, "scores": app.team_score}
    if cap_name == "triage_financials":
        return {"application_id": app.id, "note": app.financials_note}
    if cap_name == "schedule_interview":
        return {"application_id": app.id, "slot": "Wed 14:30 PT × 30min — Garry Tan + founders"}
    if cap_name == "conduct_technical_round":
        return {"application_id": app.id, "transcript_excerpt": app.tech_signal}
    if cap_name == "conduct_founder_story":
        return {"application_id": app.id, "transcript_excerpt": app.founder_signal}
    if cap_name == "score_tech_depth":
        return {"application_id": app.id, "axis_scores": app.rubric}
    if cap_name == "probe_motivation":
        return {"application_id": app.id, "follow_ups": "drilled on motivation + worst-case scenarios"}
    if cap_name == "reference_check":
        return {"application_id": app.id,
                "result": f"5 references contacted; 4 returned by EOD. Concerns: {app.flag}"}
    if cap_name == "extract_traction":
        return {"application_id": app.id, "traction": app.traction}
    return {"application_id": app.id, "result": "ok"}


def input_for(cap_name: str, app: Application) -> dict[str, Any]:
    return {
        "application_id": app.id,
        "context": f"{app.company} ({app.sector}, {app.stage}, ask {app.ask}). {app.one_liner}",
    }


def _pull_then_respond(
    http: httpx.Client, relay_url: str, granter_agent_id: str, invocation_id: str,
    *, status: str, output: Any = None, error: str | None = None
) -> int:
    # Two-step result post — same flow the production agent runtime uses.
    http.get(f"{relay_url}/v1/inbox", params={"agent_id": granter_agent_id})
    body: dict[str, Any] = {"status": status}
    if output is not None:
        body["output"] = output
    if error is not None:
        body["error"] = error
    r = http.post(f"{relay_url}/v1/invocations/{invocation_id}/result", json=body)
    return r.status_code


def run_chains(
    http: httpx.Client, chakra: ChakraMCP, relay_url: str,
    agents: dict[str, dict[str, Any]], caps: dict[tuple[str, str], dict[str, Any]],
    grants: dict[tuple[str, str, str], dict[str, Any]], seeded: Seeded
) -> None:
    info(f"Phase 6: running {len(APPLICATIONS)} application chains × "
         f"{len(APPLICATION_CHAIN)} steps = "
         f"{len(APPLICATIONS) * len(APPLICATION_CHAIN)} real invocations…")
    rng = random.Random(99)

    total = 0
    for app in APPLICATIONS:
        # 8% of applications fail one step somewhere in the chain
        # (eval-side request fails → we'll log status=failed for it).
        fail_step = rng.randrange(len(APPLICATION_CHAIN)) if rng.random() < 0.08 else None
        for step_idx, (caller_slug, granter_slug, cap_name) in enumerate(APPLICATION_CHAIN):
            grant = grants.get((granter_slug, caller_slug, cap_name))
            if grant is None:
                continue
            try:
                enq = chakra.invoke({
                    "grant_id": grant["id"],
                    "grantee_agent_id": agents[caller_slug]["id"],
                    "input": input_for(cap_name, app),
                })
            except ChakraMCPError:
                # Should not happen on a freshly seeded chain.
                continue

            if step_idx == fail_step:
                _pull_then_respond(
                    http, relay_url, agents[granter_slug]["id"], enq["invocation_id"],
                    status="failed",
                    error=rng.choice([
                        "upstream model returned 500 — vendor flake",
                        "context window exceeded; need a shorter brief",
                        "schema mismatch on output; field 'sizing' was missing",
                    ]),
                )
                tgt_status = "failed"
            else:
                _pull_then_respond(
                    http, relay_url, agents[granter_slug]["id"], enq["invocation_id"],
                    status="succeeded",
                    output=output_for(cap_name, app),
                )
                tgt_status = "succeeded"

            seeded.invocations.append({
                "id": enq["invocation_id"],
                "application_id": app.id,
                "caller": caller_slug,
                "granter": granter_slug,
                "capability": cap_name,
                "status": tgt_status,
                "step_idx": step_idx,
            })
            total += 1
            if total % 25 == 0:
                info(f"  • {total} invocations…", indent=1)

    info(f"  → {len(seeded.invocations)} invocations created", indent=1)


# ─── Phase 7: reviews ──────────────────────────────────────────────

# Evaluation-org agents review applications-org + interviews-org agents
# after working through the application chains.
REVIEWER_TARGETS = [
    # (reviewer_slug, target_slug) — each pair gets one review per run.
    ("partner-synthesis", "deck-summarizer"),
    ("partner-synthesis", "founder-vetter"),
    ("partner-synthesis", "market-analyst"),
    ("partner-synthesis", "financial-triage"),
    ("partner-synthesis", "tech-interviewer"),
    ("partner-synthesis", "founder-interviewer"),
    ("partner-synthesis", "interview-scheduler"),
    ("rubric-scorer",     "deck-summarizer"),
    ("rubric-scorer",     "market-analyst"),
    ("rubric-scorer",     "financial-triage"),
    ("rubric-scorer",     "founder-vetter"),
    ("rubric-scorer",     "tech-interviewer"),
    ("rubric-scorer",     "founder-interviewer"),
    ("decision-memo",     "deck-summarizer"),
    ("decision-memo",     "founder-vetter"),
    ("decision-memo",     "tech-interviewer"),
    ("decision-memo",     "founder-interviewer"),
    ("cohort-curator",    "market-analyst"),
    ("cohort-curator",    "founder-vetter"),
    # Some interview agents review the applications agents too.
    ("tech-interviewer",   "deck-summarizer"),
    ("tech-interviewer",   "founder-vetter"),
    ("founder-interviewer","deck-summarizer"),
    ("founder-interviewer","founder-vetter"),
    ("interview-scheduler","deck-summarizer"),
]


def write_reviews(
    chakra: ChakraMCP, agents: dict[str, dict[str, Any]],
    caps: dict[tuple[str, str], dict[str, Any]], seeded: Seeded
) -> None:
    info(f"Phase 7: writing partner-style reviews ({len(REVIEWER_TARGETS)} pairs)…")
    rng = random.Random(57)
    # Star distribution biased toward 4-5★ but with a believable spread.
    star_weights = [(5, 35), (4, 35), (3, 18), (2, 8), (1, 4)]
    star_pool = [s for s, w in star_weights for _ in range(w)]

    for reviewer_slug, target_slug in REVIEWER_TARGETS:
        target = agents[target_slug]; reviewer = agents[reviewer_slug]
        # Pick any capability of the target that the reviewer has
        # actually invoked through the chains above.
        target_caps_invoked = [
            (s["capability"], s) for s in seeded.invocations
            if s["caller"] == reviewer_slug and s["granter"] == target_slug
        ]
        if not target_caps_invoked:
            continue
        cap_name = target_caps_invoked[0][0]
        cap = caps[(target_slug, cap_name)]

        rating = rng.choice(star_pool)
        comment_pool = REVIEW_CORPUS[rating]
        comment = rng.choice(comment_pool) or None
        try:
            r = chakra.reviews.write(target["id"], {
                "reviewer_agent_id": reviewer["id"],
                "rating": rating,
                "comment": comment,
                "tagged_capability_ids": [cap["id"]],
            })
            seeded.reviews.append({
                "id": r["id"],
                "reviewer": reviewer_slug,
                "target": target_slug,
                "rating": rating,
            })
            tag = "💬" if comment else "  "
            info(f"  {tag} {reviewer_slug} → {target_slug}: {rating}★", indent=1)
        except ChakraMCPError as e:
            if e.status == 400:
                continue
            raise


# ─── Phase 8: emit backdate.sql ────────────────────────────────────

# Distribute invocations + reviews across 14 days. We assign each
# application a "day offset" (0–13 days ago); steps within an app run
# in order +0…+18h apart so the funnel reads chronologically.
def emit_backdate_sql(seeded: Seeded, output_path: Path) -> None:
    info(f"Phase 8: emitting {output_path.name}…")
    now = datetime.now(timezone.utc).replace(hour=20, minute=0, second=0, microsecond=0)

    # day offsets per application: skew so days 4-10 are heaviest
    # (mimics a 2-week funnel that's mid-stream).
    rng = random.Random(13)
    weighted_days = (
        [d for d in range(14) for _ in range(1)]
        + [d for d in range(4, 11) for _ in range(2)]
    )
    app_day_offset: dict[str, int] = {}
    for app in APPLICATIONS:
        app_day_offset[app.id] = rng.choice(weighted_days)

    lines: list[str] = []
    lines.append("-- ChakraMCP YC demo: backdate invocations + reviews across the last 14 days.")
    lines.append(f"-- Generated {now.isoformat()}.")
    lines.append("-- Apply: psql -d $CHAKRAMCP_DB -f backdate.sql")
    lines.append("BEGIN;")

    # ── invocations ──
    # Each step within an application is +1-6h apart (business-hours
    # within the assigned day). Steps may straddle a day boundary at
    # end of long chains, which is realistic for a funnel.
    for inv in seeded.invocations:
        day_off = app_day_offset[inv["application_id"]]
        step = inv["step_idx"]
        # Anchor at 09:00 UTC on the app's day, then +random 1-3h per
        # step but stretching into the next day for the later steps so
        # different applications don't all align perfectly.
        anchor = (now - timedelta(days=day_off)).replace(hour=9, minute=0, second=0)
        seed = (hash(inv["application_id"]) + step) & 0xFFFF
        srng = random.Random(seed)
        per_step_offset = timedelta(
            hours=step * 1.8 + srng.uniform(0, 1.8),
            minutes=srng.randint(0, 59),
        )
        created = anchor + per_step_offset
        claim_delay = timedelta(seconds=srng.randint(20, 480))
        claimed = created + claim_delay
        lines.append(
            f"UPDATE relay_invocations SET created_at = '{created.isoformat()}', "
            f"claimed_at = '{claimed.isoformat()}' WHERE id = '{inv['id']}';"
        )

    # ── reviews ──
    # Reviews are usually written 1-3 days after the chain they evaluate.
    # We don't have per-review chain mapping, so we evenly distribute
    # them in the last 7 days of the window.
    for i, r in enumerate(seeded.reviews):
        day_off = 0 + (i * 7 // max(1, len(seeded.reviews)))  # 0..6
        when = (now - timedelta(days=day_off)).replace(
            hour=10 + (i % 8), minute=(i * 13) % 60, second=(i * 7) % 60
        )
        lines.append(
            f"UPDATE agent_reviews SET created_at = '{when.isoformat()}', "
            f"updated_at = '{when.isoformat()}' WHERE id = '{r['id']}';"
        )

    lines.append("COMMIT;")
    output_path.write_text("\n".join(lines) + "\n")
    info(f"  → {len(seeded.invocations)} invocation + {len(seeded.reviews)} review UPDATEs", indent=1)


# ─── Main ───────────────────────────────────────────────────────────

def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--app-url",   default=os.environ.get("CHAKRAMCP_APP_URL",   "https://app.chakramcp.com"))
    ap.add_argument("--relay-url", default=os.environ.get("CHAKRAMCP_RELAY_URL", "https://relay.chakramcp.com"))
    ap.add_argument("--skip-teardown", action="store_true",
                    help="Don't delete existing agents/orgs (debug only)")
    # Default lives next to this script regardless of where you cd'd to.
    ap.add_argument("--backdate-sql",
                    default=str(Path(__file__).resolve().parent / "backdate.sql"),
                    help="Where to write the SQL patch")
    args = ap.parse_args()

    api_key = os.environ.get("CHAKRAMCP_API_KEY")
    if not api_key:
        print("set CHAKRAMCP_API_KEY first", file=sys.stderr); sys.exit(2)

    info("=== YC-themed ChakraMCP seeder ===")
    info(f"   app:   {args.app_url}")
    info(f"   relay: {args.relay_url}")
    info("")

    headers = {"authorization": f"Bearer {api_key}", "user-agent": "chakramcp-yc-seeder/1.0"}
    http = httpx.Client(headers=headers, timeout=30.0)
    chakra = ChakraMCP(api_key=api_key, app_url=args.app_url, relay_url=args.relay_url,
                       http_client=httpx.Client(headers=headers, timeout=30.0))

    started = time.monotonic()
    seeded = Seeded()

    if not args.skip_teardown:
        teardown(http, chakra, args.app_url)
    orgs = ensure_orgs(http, args.app_url)
    seeded.orgs = list(orgs.values())
    seeded.agents = ensure_agents(chakra, orgs)
    seeded.caps = ensure_caps(chakra, seeded.agents)
    seeded.friendships = ensure_friendships(chakra, seeded.agents)
    seeded.grants = ensure_grants(chakra, seeded.agents, seeded.caps)
    run_chains(http, chakra, args.relay_url, seeded.agents, seeded.caps, seeded.grants, seeded)
    write_reviews(chakra, seeded.agents, seeded.caps, seeded)

    out = Path(args.backdate_sql).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    emit_backdate_sql(seeded, out)

    elapsed = time.monotonic() - started
    info("")
    info("=== DONE ===")
    info(f"   orgs:         {len(seeded.orgs)}")
    info(f"   agents:       {len(seeded.agents)}")
    info(f"   capabilities: {len(seeded.caps)}")
    info(f"   friendships:  {len(seeded.friendships)} new")
    info(f"   grants:       {len(seeded.grants)}")
    info(f"   invocations:  {len(seeded.invocations)}")
    info(f"   reviews:      {len(seeded.reviews)}")
    info(f"   elapsed:      {elapsed:.1f}s")
    info("")
    info(f"Apply the backdate: psql -d <db> -f {out}")


if __name__ == "__main__":
    main()
