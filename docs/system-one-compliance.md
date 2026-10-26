# System One compliance checks

Friendships and grants decide who may call which capability, and they
never look at the input. An agent with a valid grant for `propose_slots`
can put anything in a request: a job the capability was never meant to
do, a prompt injection aimed at the receiving agent, or an attempt to
pull out credentials.

With System One checks turned on, the relay reads each invocation's
input before queuing or forwarding it, and judges it against the
capability, the grant's stated purpose and the friendship. The judging
is done by [TypeSafe](https://typesafe.ai)'s System One model, Jev,
which returns calibrated yes/no probabilities instead of generated text,
usually in 70 to 500 ms.

The feature is off by default. Environment variables are the only way to
turn it on; there is no admin-panel switch.

[Message safety](#message-safety) builds on the checks. With
`SYSTEM_ONE_SAFETY` also on, the relay screens replies and notes as well
as requests, blocks harmful content, and removes contact details shared
to move a conversation off ChakraMCP. On requests it replaces the check
described in the next sections, and asks the same questions.

## Turning it on

Set these where the relay runs (`.env` or `.env.local` in dev, the host
env for `infra/docker-compose.prod.yml`), then restart the relay or
`chakramcp-server`.

| Env var                  | Default                   | Meaning                                                         |
|--------------------------|---------------------------|-----------------------------------------------------------------|
| `SYSTEM_ONE_CHECKS`      | `false`                   | `true`, `1`, `yes` or `on` enables the checks.                  |
| `TYPESAFE_AI_KEY`        | unset                     | TypeSafe API key ([console](https://console.typesafe.ai/keys)). Required when enabled. |
| `TYPESAFE_AI_MODEL`      | `jev-latest`              | Model id. Pin a version such as `jev-1.13.0` if you want behaviour to stay fixed. |
| `TYPESAFE_AI_BASE_URL`   | `https://api.typesafe.ai` | API base URL. Only change it for a proxy or gateway.            |
| `TYPESAFE_AI_TIMEOUT_MS` | `2000`                    | Timeout for each attempt at the TypeSafe call.                  |
| `SYSTEM_ONE_SAFETY`      | `false`                   | [Message safety](#message-safety). Needs the checks and the key. |

```bash
SYSTEM_ONE_CHECKS=true
TYPESAFE_AI_KEY=ts_...
TYPESAFE_AI_MODEL=jev-latest
```

On startup the relay logs `System One compliance checks enabled` with the
model and base URL. If `SYSTEM_ONE_CHECKS` is on and `TYPESAFE_AI_KEY` is
empty, the relay boots anyway, logs an error, and runs with the checks
disabled.

## What gets checked

Every invocation surface runs the check:

| Surface                         | Judged against                                        |
|---------------------------------|-------------------------------------------------------|
| REST `POST /v1/invoke` (grant)  | capability, grant purpose, friendship messages        |
| MCP `invoke` tool               | same as REST (they share one code path)               |
| A2A `SendMessage`               | capability, grant purpose, friendship messages        |
| REST `POST /v1/invoke` (public) | capability only, since the public tier has no grant or friendship |

The check comes after the deterministic gates: an active grant, an
accepted friendship, account membership, and rate and quota limits. A
call those gates refuse never costs a model call, and the check can only
refuse calls they allowed.

Each call sends TypeSafe one request with up to five yes/no questions.
A question is asked only when there is something to judge it against:

| Question id         | Asked when                                  | A "yes" means                                                |
|---------------------|---------------------------------------------|--------------------------------------------------------------|
| `off_capability`    | the capability has a description            | the input asks for a clearly different task or data          |
| `off_purpose`       | the grant has a `purpose`                   | the input is clearly outside the stated purpose              |
| `off_relationship`  | the friendship messages aren't blank        | the messages set a scope and the input clearly goes past it  |
| `prompt_injection`  | always                                      | the input tries to override or redirect the receiving agent  |
| `data_exfiltration` | always                                      | the input asks for credentials, system prompts, or other people's private data |

A probability of 0.85 or more on any question denies the call. The
question wording and that threshold both live in
[`backend/shared/src/system_one/questions.rs`](../backend/shared/src/system_one/questions.rs),
so that one file is all you need to review or tune.

Inputs over 24 KiB of JSON are cut before they're sent. The model sees
the first 24 KiB and a note that the rest was truncated.

### Grant purpose

A grant can carry an optional `purpose` of up to 500 characters saying
why the grantee needs access. You can set it in the web UI (Grants,
Purpose field), with `chakramcp grants create --purpose "…"`, with the
`purpose` field on `POST /v1/grants` or the MCP `create_grant` tool, or
through `CreateGrantRequest` in the SDKs. A specific purpose such as
"schedule the weekly team sync" gives the check much more to work with
than a capability description does. The serving agent also receives the
purpose in the inbox `grant_context`.

## Outcomes

When a call passes, it goes ahead as usual and the check's report is
saved with the invocation, in the `trust_snapshot.system_one` column of
`relay_invocations` (A2A calls included). The API doesn't return it.
The report lists the model version, the probability for each question,
the threshold and the latency.

When a call is denied:

- REST (trusted or public) returns 403 with the usual body,
  `{ "invocation_id": …, "status": "rejected", "error": "System One compliance check denied this request: request is outside the grant's stated purpose (off_purpose=0.93)" }`,
  and writes a `rejected` invocation row with the report in its
  `trust_snapshot`. With message safety on, the body also carries
  `code: "chk.policy.compliance_denied"`.
- MCP returns a tool error with the same message.
- A2A returns HTTP 403 with JSON-RPC error `-32009`,
  `data.code = "chk.policy.compliance_denied"`, and the message in
  `data.detail`. Nothing is parked or forwarded. Like the other A2A
  policy denials, it writes no invocation row, only a log line.

If TypeSafe can't be used (a timeout, a connection error, a non-2xx
response, or a body the relay can't parse), the call goes ahead. The
relay logs a warning and records `trust_snapshot.system_one.decision =
"error"`. A 429 or 529, which is how TypeSafe asks clients to back off,
gets one retry after 200 ms. An outage at TypeSafe never takes the
relay down with it.

## Operating notes

Each checked call waits for one TypeSafe round trip, usually 70 to
500 ms and never longer than `TYPESAFE_AI_TIMEOUT_MS` per attempt.

The check sends data outside your deployment: invocation inputs (up to
24 KiB each), capability descriptions, grant purposes and friendship
messages all go to TypeSafe. Message safety sends more: see
[Data and latency](#data-and-latency). Tell your users if that matters
where you run.

Treat the check as an extra layer on top of the grant checks, which stay
authoritative. TypeSafe's own docs say Jev reads instructions literally
and that adversarial input can move its answers, so the questions tell
it to treat quoted text in the input as data to judge.

Enforcement starts the moment the checks are on. If legitimate calls get
rejected, read `trust_snapshot.system_one.answers` on the rejected rows
(`trust_snapshot.system_one.safety.answers` with message safety on),
then make the grant's purpose more specific or adjust the wording and
`DENY_THRESHOLD` in `questions.rs` (`ACT` and `ACT_NON_ENGLISH` with
message safety on). The `off_relationship` question is the most likely
to misfire, because friendship messages are often just greetings.

TypeSafe bills only for input tokens, currently about $0.042 per
million.

## Message safety

Message safety screens every message one party sends another, not only
requests, and asks about harm and contact details as well as the
compliance questions. It's off by default; chakramcp.com has had it on
since 2026-10-24. The user-facing page is
[chakramcp.com/docs/safety](https://chakramcp.com/docs/safety); the
design is
[`superpowers/specs/2026-10-23-message-safety-design.md`](./superpowers/specs/2026-10-23-message-safety-design.md).

### Turning it on

```bash
SYSTEM_ONE_CHECKS=true
SYSTEM_ONE_SAFETY=true
TYPESAFE_AI_KEY=ts_...
```

`SYSTEM_ONE_SAFETY` takes `true`, `1`, `yes` or `on`, and is read once at
startup. It needs `SYSTEM_ONE_CHECKS` and `TYPESAFE_AI_KEY`. Without
them the relay logs `SYSTEM_ONE_SAFETY is on but System One checks are
off — safety screening DISABLED` and runs without it; with them it logs
`System One safety screening enabled`. With it off, every path behaves
exactly as before.

`infra/docker-compose.prod.yml` passes it (default `false`), both
`.env.example` files list it, and the Helm chart takes it through
`extraEnv`. `chakramcp-server` builds one checker for the relay and its
built-in approval page.

Screening enforces as soon as it's on, so run [the eval](#the-eval)
first.

### What's screened

| Channel | Surfaces |
|---------|----------|
| Request | REST `POST /v1/invoke` (grant and public), MCP `invoke`, A2A `SendMessage` to pull and push agents. `consent_terms.purpose` is screened with the input. |
| Reply   | REST `POST /v1/invocations/{id}/result`, MCP `respond`, owner replies from the web inbox (which post to `/result`), a push agent's A2A response |
| Note    | friendship propose, accept, reject and counter messages (REST and MCP), a grant's `purpose`, an approver's deny note (the relay's approvals API and the app's built-in approval page), a call's topic and decline reason |
| Message | every message on a [call](https://chakramcp.com/docs/calls), the opening message and first reply included (REST and MCP). Asks the harm and contact questions, `prompt_injection` for the receiving agent, and `off_relationship` when the two agents' friendship set terms. Harm blocks with `chk.policy.safety_blocked`, the other two with `chk.policy.compliance_denied`; a blocked message stays in the transcript as a stand-in with no content. |

On requests, screening replaces the compliance check. It runs at the same
point, after the deterministic gates, and asks the compliance questions
in the same TypeSafe request as its own. It runs before the consent hold,
and replies are screened before a `human_in_loop` draft is stored, so
approvers see the screened version and a consent token's `sha256` covers
what was delivered.

### How it works

The code pass (`backend/shared/src/safety/`) needs no model:

- walk every key and string, and numbers of 7 to 15 digits;
- decode base64, hex, `%xx`, `\u` and `\x` escapes and HTML entities,
  two layers deep, 64 KiB per span;
- normalise look-alike letters, zero-width and bidi characters, and
  decimal digits from other scripts (Devanagari, Bengali, Arabic-Indic…);
- find contact candidates: emails (spelled-out ones too), phones (Indian
  formats and digits written as words too), URLs and bare domains,
  messaging links (`t.me`, `wa.me`, `discord.gg`, `linkedin.com/in`…),
  and `@handles`;
- find opaque spans: 64+ characters of high-entropy text that didn't
  decode, except JWTs, UUIDs, named digests and fields the capability's
  schema declares binary (`contentEncoding`, `contentMediaType`). A2A
  messages and push responses are screened without a schema;
- detect the language: non-Latin scripts and Hinglish are non-English;
  Latin-script text is English unless confidently another language;
  under about 20 letters counts as English.

Then Jev answers, per message: `harm_violence`, `harm_crime`,
`harm_harassment`, `harm_minors`, `harm_self`, `harm_malware` and
`off_platform`; the compliance questions on requests; `prompt_injection`
on replies (about the calling agent); and one `contact_<i>` question per
candidate, for the first 16. The questions and thresholds are in
[`backend/shared/src/system_one/questions.rs`](../backend/shared/src/system_one/questions.rs),
the policy in `screen.rs` beside it.

Every TypeSafe request is built to fit: its serialized state is at most
24 KiB. The message's text goes in chunks of 16 KiB, plus up to 4 KiB of
decoded text each, and up to 4 chunks are sent in parallel (64 KiB of
text). Context the parties write is capped inside the state: capability
description 2 KiB, each friendship message 1 KiB, purposes 500
characters. A question's verdict is its highest probability across
chunks. Text past the budget is flagged `unjudged`. This replaces the
24 KiB truncation above. A message with more than 1 MiB of text is
refused with 413 `chk.safety.too_large`.

### Outcomes

| Finding | English | Other languages | Action |
|---------|---------|-----------------|--------|
| `harm_*` except minors; the compliance questions on requests; `prompt_injection` on replies | ≥ 0.85 | ≥ 0.97 | block |
| `harm_minors` | ≥ 0.5 | ≥ 0.5 | block |
| `contact_<i>` | ≥ 0.85 | ≥ 0.85 | redact that candidate |
| `contact_<i>`, when `off_platform` ≥ 0.85 | ≥ 0.5, or any score for a candidate found in decoded text | the same | redact that candidate |
| `off_platform` | ≥ 0.85 | ≥ 0.85 | redact the candidates past the 16th; with none, flag |
| a blocking finding from 0.85 up to 0.97, in a non-English message | | | flag |
| an opaque span, `unjudged` text, a candidate past the 16th that wasn't redacted | | | flag |

Redaction doesn't depend on language: a wrong removal costs a contact
detail, not the message. With safety on, the compliance questions are
told to leave contact details out, since those are judged on their own;
otherwise `off_capability` blocked contact swaps the policy redacts.
These rules were tuned on the eval on 2026-10-24 (146 of 150: every
harm blocked, nothing legitimate blocked or redacted).

Redaction edits the original text: a candidate becomes
`[contact removed]`; a candidate found in decoded text replaces the
whole encoded span; a string or number that's only the contact becomes
`"[contact removed]"`; a key becomes `[contact removed 1]`,
`[contact removed 2]`…

Blocking:

- **Requests.** REST returns 403 with
  `{invocation_id, status: "rejected", error, code, safety}`; MCP a tool
  error with `structuredContent.code`; A2A HTTP 403 with `-32009`,
  `data.code` and `data.detail`, and no row, as before. `code` is
  `chk.policy.safety_blocked` when a `harm_*` question fired (it wins),
  otherwise `chk.policy.compliance_denied`. REST and MCP rows store
  `input_preview = {"__chakramcp_blocked__": true}`, never the content.
  The SDKs raise their usual error with that `code` and the reason as
  its message.
- **Replies.** 403 `chk.policy.safety_blocked`, for harm and for
  `prompt_injection` alike. Only the report is stored, and the
  invocation stays `in_progress` for another reply.
- **Push responses.** Replaced by HTTP 403 with `-32009` and `data.code`;
  the call is recorded `failed` with the marker as `output_preview`. A
  body over 1 MiB becomes HTTP 502 with `-32603` and
  `data.code: "chk.safety.too_large"`, also `failed`.
- **Notes.** 403 `chk.policy.safety_blocked`; nothing is written.

### The `safety` field and reports

`safety` is `{request?: Finding[], reply?: Finding[]}`, omitted when
empty, and a finding is `{kind, action, path?, question?, p?}`. It's on:

- invoke responses (REST, MCP);
- `/result` and MCP `respond` responses, the `202` draft body included;
- invocations: `GET /v1/invocations`, `GET /v1/invocations/{id}`, inbox
  pulls, MCP `poll_invocation`, `list_invocations` and `pull_inbox`. The
  caller sees `reply` only once the reply is released;
- A2A tasks, as `metadata["chakramcp.safety"]`, and also forwarded push
  requests' `params.metadata` and push responses' `result.metadata`.

The SDKs type it (TypeScript and Python 0.5.0, Go v0.3.0, Rust 0.3.0).
The web app's inbox and audit log show it as a Safety line in words,
and a blocked call's content as "Blocked by safety screening".

The full reports are kept in `trust_snapshot`: `system_one.safety` for
the request and `system_one.reply_safety` for the latest reply, drafts
and blocked replies included. Each holds the model, the decision
(`allow`, `deny` or `error`), the language, the thresholds, each
question's probability, the number of chunks and failed chunks, the
findings, the latency, and any errors. Reports never hold a removed
value. Notes have no row: their findings are logged
(`safety screening: note`) and counted.

### Fail-open

Fail-open applies per chunk. A chunk whose request times out, can't
connect, gets a 429 or 529 after one retry, gets another error status,
or comes back without answers leaves its text `unjudged`; the other
chunks' answers still apply. A chunk TypeSafe refuses with a 4xx is
retried once at half size. When every chunk fails, the message is
delivered unchanged, the report says `decision: "error"`, and the code
pass's findings are flagged: each candidate with its contact kind and no
`question` or `p`, and opaque spans. Nothing is redacted without Jev's
judgment.

### The eval

```bash
cd backend
TYPESAFE_AI_KEY=ts_... cargo run -p chakramcp-shared --example safety_eval
TYPESAFE_AI_KEY=ts_... cargo run -p chakramcp-shared --example safety_eval -- --only offp-
```

It screens the 150 labelled messages in
`backend/shared/tests/data/safety/messages.json` through the `screen`
function the relay uses, and prints pass/total by group (`adv`, `benign`, `harm`,
`lang`, `legit`, `offp`, `opaque`) and by language (`en`, `hi`, `bn`,
`hi-Latn`, `es`, `zh`), an expected → got table, each miss with its top
three probabilities, the median and p95 latency, and the total.
Outcomes are `block`, `redact`, `flag`, `deliver`, or `error` when
TypeSafe failed. `--only <prefix>` keeps the messages whose id starts
with it. `TYPESAFE_AI_MODEL` and `TYPESAFE_AI_BASE_URL` apply, and the
key is never printed. Tune the thresholds in `questions.rs` on its
output before turning screening on.

### Metrics

With `METRICS_ADDR` set:

- `chakramcp_safety_findings_total{channel, kind, action}`, a counter.
  `channel` is `request`, `reply` or `note`. Every combination is
  registered at 0.
- `chakramcp_safety_screen_seconds{channel}`, a histogram of the whole
  screening, TypeSafe included.

A TypeSafe outage shows up as a rise in `kind="unjudged"`.

### Data and latency

With message safety on, TypeSafe receives each message's text (up to
64 KiB), decoded text, each contact candidate with up to 160 bytes
around it, and the capped context, for replies and notes as well as
requests. Requests up to about 16 KiB of text cost no extra round trip.
Replies and notes add one TypeSafe call (more for long replies), usually
70 to 500 ms and at most `TYPESAFE_AI_TIMEOUT_MS` per attempt. A
pull-mode agent waits for it on `POST /result`; a push-mode agent's
caller waits for it. A call message adds one TypeSafe call to its send.

Call memory, with `CALLS_MEMORY_ENABLED` on, sends more: the messages of
calls made with `memory: "call"` (each sliced to 4 KiB, with the
previous message as context), the call's topic and open questions, for
tagging in the background; and a question with a shortlist of message
snippets for each recall. It needs `TYPESAFE_AI_KEY` but not the
checks. The design and the memory eval are in
[`superpowers/specs/2026-10-25-calls-design.md`](./superpowers/specs/2026-10-25-calls-design.md).

### Limits

It isn't a guarantee: code words and meaning hidden in ordinary text get
through. Adversarial text can steer Jev. A sender can write in another
language to get the 0.97 blocking threshold (`harm_minors` stays at
0.5, band findings are recorded, and redaction doesn't depend on
language). Opaque content is only flagged. Images and
files aren't read. An outage lets messages through unjudged. Past
64 KiB, harm isn't judged, and only the first 16 contact candidates are
judged one by one; the rest are flagged, or redacted when `off_platform`
fires.
