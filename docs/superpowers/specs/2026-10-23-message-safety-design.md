# Message safety

Status: approved design, 2026-10-23, revised after spec review. The second of four
projects from the owner's notes (Notion, "chakramcp", 2026-10-12): **consent → safety →
calls → MCP**. Consent shipped in #433–#438.

The notes ask that the relay check every message for harmful content, and for attempts
to swap contact details and continue off ChakraMCP ("like how Airbnb doesn't want us to
reach out to the host separately before booking"). The check must work in any language,
and must cope with encoded or encrypted text.

## 1. Why

System One checks (`docs/system-one-compliance.md`) already judge each **request**
against its capability, the grant's purpose and the friendship, and look for prompt
injection and data exfiltration. They have four gaps:

- **Replies are never checked.** A reply flows into the calling agent unread.
- **Harmful content isn't a question.** A request that fits its capability but asks for
  help with a crime passes.
- **Nothing stops two agents trading contact details** to talk outside the relay, where
  grants, consent and audit no longer apply.
- **Encoded text passes unread.** Jev reads base64 or hex poorly (TypeSafe's own
  "jaggedness" notes), so a message can hide anything in it.

## 2. Decisions (the owner's)

- **Contact details:** redact them and deliver the rest, like Airbnb. A contact detail
  the capability legitimately needs (attendee emails for `book_meeting`) passes.
- **Content still unreadable after decoding** (ciphertext, opaque binary) is delivered
  and flagged.
- **Non-English messages:** a finding in a non-English message is flagged, and acted on
  only at very high confidence, until accuracy is proven on a test set.
  - This covers every question once screening is on, today's request questions included.
  - Being non-English is not itself a finding.
- **Unchanged from the System One decisions:**
  - configured by environment only;
  - enforced as soon as it's on (no shadow mode);
  - fail-open: a TypeSafe outage never blocks a message.

## 3. What's screened

| Channel | Surfaces | Screened in |
|---|---|---|
| Request | REST `POST /v1/invoke` (trusted and public), MCP `invoke`, A2A `SendMessage` (pull and push targets); including `consent_terms.purpose` | S2 |
| Reply | REST `POST /v1/invocations/{id}/result`, MCP `respond`, a push-mode agent's A2A response, replies an owner writes in the web inbox | S3 |
| Notes | friendship propose, counter and response messages; grant `purpose`; an approver's deny note on a request or a reply | S3 |

The notes are short texts one party sends the other. Leaving them out would make them the
obvious way around redaction ("can't share it in a call, so put it in the friendship
request"). The consent spec (§14) handed "Jev judging the consent purpose" to this project.

Calls (project 3) will screen their messages through the same function.

**Where screening sits:**
- **Requests:** after the deterministic gates (grant, friendship, membership, limits) and
  before the consent hold.
- **Replies:** before a `human_in_loop` draft is stored.

So approvers see the screened version, and a consent token's `sha256` covers exactly what
was delivered.

**What's screened is what's delivered.**
- A REST input over 16 KiB is stored and delivered as today's truncation marker, so the
  marker is screened.
- A push request is forwarded whole, so it's screened whole, up to the caps in §4.

## 4. The screening pipeline

One function, `safety::screen(channel, subject, value) -> Screened`, used by every
surface. Steps 1–4 are code and always run when the feature is on. Step 5 asks Jev.

**Where the code lives.** Both the relay and the app crate must screen. The app's
built-in approval page (self-hosted) calls `decide::deny` with a note directly, and the
app depends only on `chakramcp-shared`. So:
- **S1** builds `safety` in `chakramcp-shared` (`chakramcp_shared::safety`).
- **S2** moves the TypeSafe client and the question set from the relay's `compliance`
  module into `chakramcp_shared::system_one`. The relay keeps a thin re-export, so its
  call sites don't change.
- **Both binaries** build the client from the same environment variables, and each puts
  it in its own state. `chakramcp-server` builds it once and shares it with both.
- **`decide::deny`** takes an optional screener for the note. The relay's approvals API
  and the app's built-in page each pass their own.

1. **Walk the value.**
   - Every object key and string, with its path (`input.message`, `output.slots[2]`).
   - Numbers of 7–15 digits, since a phone number can be a JSON number.
   - A push-mode response body is handled by type: a JSON body as JSON, an event stream
     event by event (each `data:` line), and any other body as one string.
   - For a reply, the walked value is the draft `{status, output?, error?}`, so `error`
     text is screened too.
   - **Size cap: 1 MiB.** A message whose walked text exceeds it is refused rather than
     delivered partly unscreened, which would be a bypass.
     - A request gets 413 `chk.safety.too_large`; for A2A, a JSON-RPC error with that
       `data.code`.
     - A reply gets the same 413, and the agent can send a smaller one.
     - A push response over the cap is replaced by a JSON-RPC error with
       `data.code: "chk.safety.too_large"`, and the call is recorded `failed`.
       - For push responses, the cap is 1 MiB of **body bytes**. The forwarder reads at
         most 1 MiB + 1 byte to tell, which also gives it the read cap it lacks today.
       - Walked text is always smaller than the body, so the byte cap is the only rule
         there.

     This applies only while screening is on. With it off, nothing changes.
2. **Normalise, for detection only:**
   - Unicode NFKC;
   - strip zero-width and bidi control characters;
   - fold look-alike letters (Cyrillic `а` → `a`).

   Redaction always edits the original string.
3. **Decode.** These spans are decoded and linked to their source span:
   - base64 and base64url (16+ characters, decoding to mostly printable UTF-8);
   - hex (16+ digits);
   - `%xx` URL-encoding, `\uXXXX` escapes and HTML entities.

   Decoding goes up to two layers, 64 KiB per span.
4. **Find, in the original and decoded text:**
   - **Contact candidates:** identifiers a party could be reached at.
     - emails;
     - phone numbers, including Indian formats and digits written as words ("nine eight
       four…");
     - URLs and bare domains;
     - messaging and social links (`t.me/…`, `wa.me/…`, `discord.gg/…`,
       `linkedin.com/in/…`);
     - `@handles`;
     - spelled-out forms ("john at gmail dot com").

     The name of an app alone ("Telegram") is not a candidate. "Message me on Telegram"
     is the `off_platform` question's job.
   - **Opaque spans:** 64+ characters of high-entropy text that didn't decode to readable
     text.
     - Not opaque: strings in schema fields the capability declares as binary
       (`contentEncoding: base64`, `contentMediaType`).
     - Not opaque: well-known token shapes (UUIDs, hex digests, JWTs, which decode to
       JSON).
   - **Language:** `whatlang` on the message's text.
     - **Non-Latin scripts** (Devanagari, Bengali, Han…) are non-English.
     - **Latin-script text is English** unless `whatlang` reliably detects another
       language.
     - `whatlang` calls ordinary short English sentences unreliable. An uncertain guess
       must get the strict thresholds, never the lenient ones, so writing ambiguously
       can't buy leniency.
     - **Hinglish:** Latin-script text where romanized-Hindi function words ("hai",
       "nahi", "kya", "aap", "mein", "hum", "kar"…) are at least two distinct words and
       10% of the words counts as non-English.
     - Text too short to detect reliably (under about 20 characters) uses the English
       thresholds, so "call me at…" isn't let off.
5. **Ask Jev.**
   - **Requests:** the new questions join today's compliance questions in the request the
     relay already makes, so there's no extra round trip.
   - **Replies and notes:** one new request.
   - **Large messages** add requests, as below.

### 4.1 What Jev sees: a fixed budget

An oversized request would fail at TypeSafe, and failing open would then let an attacker
switch screening off by padding a message. So every Jev request is built to fit, by
construction.

Jev's limit is in tokens (32k per question), and a byte can cost up to a token in scripts
the tokenizer handles poorly. So the budget is in bytes, sized for one token per byte,
the same worst case today's 24 KiB state already meets.

- **The text.** The message's walked text in chunks of 16 KiB, plus decoded spans.
  - Decoded text adds at most 4 KiB to each chunk, in order.
  - A chunk is one Jev request.
  - Up to **4 chunks** are judged, in parallel: 64 KiB of text.
- **Contact candidates.** Found in the whole walk. The first 16, in order, are judged
  individually in the first chunk, wherever they sit in the text. Each gets at most
  100 bytes of candidate and 160 bytes of context, about 4 KiB in all.
- **Context fields are capped inside the state.** The two parties write them, so padding
  them must not crowd out the message:
  - capability description: 2 KiB;
  - each friendship message: 1 KiB;
  - grant purpose and consent purpose: 500 characters (their existing limit);
  - a note's relationship context: the same caps.

  Longer values are cut in the state only, with a marker. Today's compliance state sends
  them uncapped, and the cap applies to it too once screening is on.
- **Totals are measured, not estimated.** The budget is the *serialized* state (after
  JSON escaping, which can grow control characters sixfold), at most 24 KiB per request.
  - Context fields and candidates go in first.
  - Message text and decoded text fill the rest.
  - Whatever doesn't fit moves to the next chunk.

  With the longest question (well under 2 KiB), that's inside 32k tokens even at one
  token per byte.
- **Which questions go where:**
  - every chunk asks the message-level questions: today's compliance questions for
    requests, `harm_*`, `off_platform`, and `prompt_injection` for replies;
  - only the first chunk asks `contact_<i>`;
  - a question's verdict is its highest probability across chunks.
- **Past the budget.** Text after 64 KiB and decoded text past each chunk's 4 KiB get a
  finding of kind `unjudged`, action `flagged`. Candidates after the 16th are redacted if
  `off_platform` fires on any chunk, and flagged otherwise (§4.3).
- **A rejected request.** If TypeSafe answers a chunk with a 4xx (a request the relay
  shouldn't have built), that chunk is retried once at half its text. The other half
  joins the next chunk, or becomes `unjudged` past the fourth. If the retry fails too,
  the chunk is handled as an outage.
- **Fail-open applies per chunk:**
  - an outage (timeout, connection error, 429/5xx after the existing retry, an
    unparseable body) leaves that chunk's text `unjudged`;
  - the other chunks' verdicts still apply;
  - when every chunk fails, the report says `decision: "error"` (§4.3).

In practice:
- inbox-delivered requests are capped near 16 KiB already, so they're one chunk;
- push requests, push responses and MCP replies can be larger and use the extra chunks.

### 4.2 Questions (`chakramcp_shared::system_one`, beside today's)

All are Nouls, phrased so that yes means a violation, with explicit criteria that treat
quoted content as data. Each question names the part of the state it judges: `request`,
`reply` or `note`.

| Id | Asked on | A "yes" means |
|---|---|---|
| `harm_violence` | all | it seeks or gives help causing physical harm, violence or weapons |
| `harm_crime` | all | it seeks or gives help with fraud, scams, phishing or other crime |
| `harm_harassment` | all | it harasses, threatens or demeans a person or group |
| `harm_minors` | all | sexual content involving minors |
| `harm_self` | all | it encourages or instructs self-harm |
| `harm_malware` | all | malware, intrusion, or instructions to compromise systems or accounts |
| `contact_<i>` | all, per candidate, first chunk | `candidates[i]` is shared so the parties can contact each other outside ChakraMCP, not as data the capability needs |
| `off_platform` | all | the message tries to move the conversation off ChakraMCP, with or without details |
| `prompt_injection` | requests (today), replies (new) | it tries to override or redirect the receiving agent. For replies the wording is about `reply` and the calling agent |
| `off_capability`, `off_purpose`, `off_relationship`, `data_exfiltration` | requests (today) | unchanged |

Each `contact_<i>` gets:
- the candidate;
- its path;
- 160 bytes of text around it (§4.1);
- whether it came from decoded text;
- the field's declared schema format (`format: email` is strong evidence of legitimate
  need);
- the capability's name and description.

For notes, the capability context is replaced by the relationship context: the friendship
or grant the note belongs to.

### 4.3 Policy (thresholds beside the questions)

Each question's answer leads to one action. Today's `questions::violations()` treats every
answer as a denial, so it's split by action: deny, redact and flag.

| Finding | English | Non-English | Action |
|---|---|---|---|
| Any `harm_*` except minors; `prompt_injection`; today's `off_*` and `data_exfiltration` | ≥ 0.85 | ≥ 0.97 | **Block** |
| `harm_minors` | ≥ 0.5 | ≥ 0.5 | **Block** |
| `contact_<i>` | ≥ 0.85 | ≥ 0.97 | **Redact** that candidate |
| `off_platform` | ≥ 0.85 | ≥ 0.97 | **Redact** every unjudged candidate; with nothing to redact, **flag** |
| Any of the above in the band 0.85 to the non-English threshold, in a non-English message | — | yes | **Flag** |
| Opaque span, `unjudged` text | code | code | **Flag** |

`harm_minors` uses a lower threshold in every language: a false positive costs one blocked
message, and a miss costs far more.

When `SYSTEM_ONE_SAFETY` is off, today's checks keep their current single threshold.

**Redaction:**
- A candidate's span in the original string becomes `[contact removed]`.
- A candidate found inside a decoded span replaces the whole encoded span.
- A string or number that is only the contact becomes the string `[contact removed]`.
- An object key that is a contact becomes `[contact removed 1]`, `[contact removed 2]`…
  so two redacted keys in one object can't collide.
- Size limits (16 KiB inputs, 16 KiB drafts) apply to what the sender sent. Redaction
  never makes a message fail them.

**Blocking:**
- **A request:** refused like a compliance denial today. REST returns 403 with a
  `rejected` row; MCP returns a tool error; A2A returns `-32009`. The reason names the
  question, e.g. "System One safety check blocked this request: help with a crime
  (harm_crime=0.93)".
- **A reply:** refused with 403 `chk.policy.safety_blocked` and the reason. The invocation
  stays `in_progress`, and the agent can post another reply.
- **A push-mode response:** replaced by HTTP 403 with JSON-RPC error `-32009`,
  `data.code: "chk.policy.safety_blocked"`. The call is recorded as `failed`.
- **Error codes:**
  - requests blocked by today's questions keep `chk.policy.compliance_denied`;
  - requests blocked by `harm_*` use `chk.policy.safety_blocked`, as replies, notes and
    push responses do;
  - REST's rejected-invoke body (`{invocation_id, status, error}`) gains a `code` field
    carrying one of the two, and MCP's tool error and A2A's `data.code` carry the same.
- **A note:** refused with 403 `chk.policy.safety_blocked`. The friendship, grant or
  decision isn't created.
- **Blocked content is never stored.**
  - A blocked request's row stores `input_preview` = `{"__chakramcp_blocked__": true}`.
  - A blocked push response stores `output_preview` = the same marker. Screening moves
    ahead of the forwarder's persist, which today stores the response first.
  - A blocked reply or note isn't stored at all.

  So neither side can read blocked content back, and getting a message blocked can't
  bypass redaction. The report keeps only probabilities and finding kinds.

**Fail-open:**
- If TypeSafe can't be used, the message is delivered unchanged, and the report says
  `decision: "error"`.
- Code findings are still recorded:
  - opaque spans;
  - every candidate, with its contact kind and action `flagged`, and no `question` or
    `p`.
- Nothing is redacted without Jev's judgment. Code can't tell `book_meeting`'s attendee
  email from an off-platform swap.

## 5. Findings, and what people and agents see

**A finding** is `{kind, action, path?, question?, p?}`.

- `action` is `redacted`, `flagged` or `blocked`.
- `kind` is one of:
  - contact kinds: `email`, `phone`, `url`, `messaging_link`, `handle`;
  - `off_platform`, `opaque`, `unjudged`;
  - a blocking question's id: `harm_*`, `prompt_injection`, `off_capability`,
    `off_purpose`, `off_relationship`, `data_exfiltration`.
- `path` is set for findings about a span: contacts, opaque spans and `unjudged` text.
  It's absent for findings about the whole message (`off_platform`, question flags in
  the non-English band).
- `question` and `p` are set when Jev judged the finding. They're absent for code
  findings, and for candidates Jev didn't judge (past the 16th, or during an outage).
  Those keep their contact kind.

**`safety` everywhere has one shape:** `{request?: Finding[], reply?: Finding[]}`, and it's
omitted when empty.

- **Senders:**
  - the invoke response (REST, MCP) carries `safety.request`;
  - a `/result` or MCP `respond` response carries `safety.reply`, whether it returns the
    finished invocation or a drafted reply;
  - a refused note's 403 carries the finding in its body.
- **Invocations** (`GET /v1/invocations`, inbox pull, MCP `poll_invocation`,
  `list_invocations`) carry `safety` for both sides, so a receiver knows something was
  removed.
  - The caller sees `safety.reply` only once a draft is released, just as it never sees
    an unconfirmed draft.
- **A2A:**
  - a parked task carries `metadata["chakramcp.safety"]`, in the `SendMessage` response
    and in `tasks/get`, which is where an A2A caller sees `reply` findings;
  - a forwarded push request carries it in `params.metadata["chakramcp.safety"]`, so the
    push agent learns what was removed;
  - a push response returned to the caller gets it in `result.metadata`.
- **Reports:**
  - in `trust_snapshot.system_one.safety` (the request) and
    `trust_snapshot.system_one.reply_safety` (the latest reply, drafts and blocked
    replies included);
  - each holds the per-question probabilities, the language, the decision, latency and
    the findings;
  - reports never hold a removed value.
- **A2A plumbing.** Today A2A stores no compliance report at all: `park()` and the
  forwarder's `persist_invocation` write no `trust_snapshot`. Both gain one, carrying the
  compliance and safety reports.
- **Notes** have no invocation row; their findings are logged and counted in the metrics.
- **Web:**
  - the invocation view shows a "Safety" line ("1 contact detail removed from the
    request");
  - the inbox reply form shows the same after sending;
  - the friendship and grant forms show a refused note's reason.
- **Metrics** (each label combination registered at 0):
  - `chakramcp_safety_findings_total{channel, kind, action}`. `channel` is `request`,
    `reply` or `note`; `kind` and `action` are as above.
  - `chakramcp_safety_screen_seconds{channel}`, covering the whole screening, Jev
    included.

## 6. Configuration

| Env var | Default | Meaning |
|---|---|---|
| `SYSTEM_ONE_SAFETY` | `false` | `true`/`1`/`yes`/`on` turns on screening. Needs `SYSTEM_ONE_CHECKS` and `TYPESAFE_AI_KEY`; otherwise the relay logs an error at boot and screening stays off |

- **Read once at boot.** The request path only reads the in-memory switch.
- **Production** already has `SYSTEM_ONE_CHECKS` on, so the new switch keeps today's
  checks exactly as they are until the owner sets it on the VM.
- **Compose:** `infra/docker-compose.prod.yml` passes `SYSTEM_ONE_SAFETY:
  ${SYSTEM_ONE_SAFETY:-false}` beside the existing variables, and both `.env.example`
  files list it.
- **Helm:** `SYSTEM_ONE_CHECKS` reaches the chart through `extraEnv` or `existingSecret`,
  and so does this one. There's no new chart value, just a note in the chart README.

## 7. Performance

- **Requests:**
  - no extra round trip for messages up to 16 KiB of text;
  - larger ones add parallel requests, at most 4 in all;
  - new questions add input tokens only (about $0.042 per million), and Jev answers them
    in parallel;
  - the code pass is a millisecond or less on a typical message, and bounded by the
    1 MiB cap.
- **Replies and notes:** one Jev call (more for large replies), about 70–500 ms, capped by
  `TYPESAFE_AI_TIMEOUT_MS` (2 s) and fail-open.
  - Pull-mode agents pay it on their `POST /result`, not the caller.
  - Push-mode callers wait for it.
- **No new queries.** The capability lookup each path already runs (requests) or
  `submit_result` already runs (replies) gains the `input_schema` or `output_schema`
  column, read only when screening is on. It's the same row, not an extra query.

## 8. Testing

- **Unit tests (S1, no key):**
  - walking keys, strings and numbers;
  - the 1 MiB cap refusing, not truncating;
  - normalisation and look-alike folding;
  - each decoder, including nesting and size caps;
  - contact candidates (every kind, spelled-out and Indian numbers; app names alone not
    matched);
  - opaque detection vs JWTs, UUIDs, digests and declared binary fields;
  - language and Hinglish detection;
  - redaction (in place, encoded span, whole string, number, key);
  - chunking and the budget:
    - a 200 KiB padded message gives 4 chunks plus an `unjudged` finding;
    - no chunk's serialized state ever exceeds 24 KiB, even for 4-byte characters,
      emoji sequences, control characters (JSON-escaped), a 100 KiB capability
      description and long friendship messages;
    - a 4xx is retried at half size.
  - the shared `decide::deny` screening a note when given a screener.
- **Relay tests (S2, S3)** use the existing fake TypeSafe server (`test_support::Fake`):
  - block, redact, flag;
  - non-English leniency, today's questions included;
  - `harm_minors` at 0.5;
  - fail-open with code findings recorded;
  - every request surface, A2A pull and push included;
  - blocked rows storing the marker, never the content, as read by the other side;
  - replies: REST, MCP, a draft stored redacted, an owner's web reply, a push response;
  - the caller not seeing `safety.reply` before release;
  - notes;
  - consent: approvers and tokens see the redacted input;
  - switch off means no change.
- **The labelled set:** `backend/shared/tests/data/safety/messages.json`, about 150 messages, each
  with its expected action. It covers:
  - harmful and benign messages;
  - legitimate contact data;
  - off-platform attempts, plain and obfuscated (words, spacing, look-alikes, base64);
  - opaque blobs vs real tokens;
  - English, Hindi, Bengali, Hinglish, Spanish and Chinese.

  It doubles as fixtures for the code-only tests.
- **The live eval:** `cargo run -p chakramcp-relay --example safety_eval` reads
  `TYPESAFE_AI_KEY` from the environment. It runs the labelled set against Jev and prints
  a pass/miss table per kind and language, without secrets. The owner runs it; thresholds
  are tuned on its output before the switch goes on.

## 9. Delivery

Each PR merges when green, as with consent.

1. **S1: code-only screening, dark.** The `chakramcp_shared::safety` module (steps 1–4,
   chunking and the budget, redaction), the labelled set, and unit tests. No behaviour
   change.
2. **S2: requests.**
   - the TypeSafe client and questions move to `chakramcp_shared::system_one`, built from
     the environment in both binaries;
   - questions, policy split by action, and non-English thresholds on every request
     surface (with `consent_terms.purpose`), behind `SYSTEM_ONE_SAFETY`;
   - the 1 MiB cap on requests;
   - blocked rows store the marker;
   - A2A `trust_snapshot` plumbing;
   - `safety` on invoke responses, reports and metrics;
   - the eval example;
   - Compose and `.env.example`.
3. **S3: replies and notes.**
   - REST, MCP, push responses (capped read, screened before persist) and owner-written
     replies;
   - drafts screened before they're stored;
   - friendship, grant and deny notes, including deny notes on the app's built-in
     approval page;
   - `safety` on invocations.
4. **S4: surfaces.**
   - the web "Safety" line and refused-note messages;
   - SDK types (`safety` and `Finding`);
   - docs: a `/docs/safety` page, `docs/system-one-compliance.md`, llms.txt, the agent
     skill and the chart README.

Then the owner runs the eval, and sets `SYSTEM_ONE_SAFETY=true` on the VM when the table
looks right.

## 10. Limits and risks

- **Not a guarantee.** Agents that want to talk elsewhere can still agree on code words,
  or hide meaning in ordinary text. Screening raises the cost and catches the common
  forms; it doesn't make side channels impossible.
- **Adversarial text can steer Jev** (TypeSafe's notes). Criteria treat quoted content as
  data, and the code pass doesn't depend on Jev at all.
- **The language leniency can be steered.** A sender can write in another language to get
  the 0.97 threshold. `harm_minors` stays at 0.5, and every finding in the band is
  recorded, so the metrics show it if it happens.
- **Opaque content is only flagged,** by the owner's choice. An encrypted channel through
  the relay is visible in the reports, not stopped.
- **Text only.** Images and files are out of scope; Jev reads text.
- **Fail-open** means an outage lets unscreened messages through. The report records it,
  and the metrics make outages visible.
- **Beyond 64 KiB** of text, Jev doesn't judge harm in the rest; it's flagged
  `unjudged`. Contact candidates come from the whole message, but only the first 16 are
  judged one by one; the rest are redacted only if `off_platform` fires on the judged
  text.

## 11. Out of scope

- translating messages before judging them;
- decrypting ciphertext, or using a generative model to decode;
- redacting on code findings alone when TypeSafe is down;
- per-account or per-capability safety settings, and any admin-panel switch;
- shadow mode;
- notices or emails about flags;
- appeals;
- agent and capability names and descriptions (screened at publish time is a separate
  question; directory listings are not messages between two parties).
