# Message safety S1: code-only screening, dark — implementation plan

Derived from `2026-10-23-message-safety-design.md` (approved 2026-10-23); § numbers refer
to that spec. This is the plan for S1 of §9. S2–S4 get their own plans once this one has
merged, because they build on its code.

S1 changes nothing anyone can see. It adds `chakramcp_shared::safety`, which nothing calls
yet, plus the labelled message set and unit tests.

**Conventions**
- **Ship flow:**
  - branch `feat/message-safety-1` from `main` → PR → green CI → squash merge (the owner's
    standing approval) → check that commit's CD run;
  - commits end with the Co-Authored-By trailer;
  - PR bodies end with the Claude Code footer.
- **Before pushing:**
  - `cd backend && cargo test --workspace`;
  - `cargo clippy --workspace --all-targets -- -D warnings`.
- **sqlx:** S1 adds no queries. The hook still needs
  `DATABASE_URL=postgres://chakramcp:chakramcp@localhost:5544/chakramcp` exported in the
  commit's shell.
- **Dependencies** (`backend/shared/Cargo.toml`):
  - `regex = "1"`, `unicode-normalization = "0.1"` and `percent-encoding = "2"` (all
    already in `Cargo.lock`);
  - `base64 = { workspace = true }`;
  - `whatlang = "0.18"`, the only new crate (MIT, 2025-10-16, no unsafe C).
- **No behaviour change:** nothing outside the module and its tests references it.

**Refinements made while planning** (the spec's intent, a more concrete mechanism):
- **Jev sees fields, not raw JSON.**
  - Each chunk's message text is a list of `{path, text}` entries (a long string is split
    into `part`s), not a slice of serialized JSON.
  - This gives every finding, `unjudged` ones included, a real path.
  - It lets the budget count exactly what's sent.
  - S2 points the questions at `request.fields` / `reply.fields` when screening is on.
- **Addressing.** Walked items carry an RFC 6901 JSON Pointer for exact edits, plus the
  display path from §5 (`input.attendees[0]`). Redaction uses the pointer.
- **Digit folding.** Normalisation also folds Unicode decimal digits (Devanagari `०–९`,
  Bengali `০–৯`, Arabic-Indic) to ASCII, so phone numbers written in Indian scripts are
  found. NFKC doesn't do this.
- **The labelled set lives at `backend/shared/tests/data/safety/messages.json`**, next to
  the module that reads it. §8 of the spec is updated to match. S2's eval example reads it
  from there.
- **No positive `harm_minors` examples in the repo.** That question's threshold mechanics
  are tested in S2 with the fake TypeSafe server, which needs no such content. The set
  only includes benign near-misses (school schedules, children's books).

---

## S1 steps

1. **Types (`safety/mod.rs`, `safety/findings.rs`, §5):**
   - `Channel { Request, Reply, Note }` (`as_str`: `request` / `reply` / `note`).
   - `Action { Redacted, Flagged, Blocked }`.
   - `FindingKind`, an enum with one variant per kind in §5. That's the contact kinds,
     `off_platform`, `opaque`, `unjudged`, and the question ids (`harm_*`,
     `prompt_injection`, `off_capability`, `off_purpose`, `off_relationship`,
     `data_exfiltration`).
     - serde in snake_case;
     - `FindingKind::ALL`, for S2's metrics registration.
   - `Finding { kind, action, path: Option<String>, question: Option<String>, p: Option<f64> }`,
     with serde that skips `None`.
   - `Safety { request: Option<Vec<Finding>>, reply: Option<Vec<Finding>> }`, the wire
     shape.
   - Constants, each named after its spec section:
     - `MAX_WALK_BYTES = 1 MiB`;
     - `MAX_DECODE_BYTES = 64 KiB`;
     - `MAX_DECODE_DEPTH = 2`;
     - `CHUNK_TEXT_BYTES = 16 KiB`;
     - `CHUNK_DECODED_BYTES = 4 KiB`;
     - `MAX_CHUNKS = 4`;
     - `MAX_JUDGED_CANDIDATES = 16`;
     - `CANDIDATE_BYTES = 100`;
     - `CONTEXT_BYTES = 160`;
     - `STATE_BUDGET_BYTES = 24 KiB`.

2. **Walk (`safety/walk.rs`, §4 step 1):**
   - `walk(value: &Value) -> Result<Vec<Item>, TooLarge>`.
   - `Item { pointer, path, at: At::{Key, Str, Num}, text }`.
   - Visits:
     - object keys and string values;
     - numbers whose integer form has 7–15 digits (phone-shaped, as text);
     - nothing else.
   - The running total of text bytes over `MAX_WALK_BYTES` returns `TooLarge`; it never
     truncates.
   - `walk_text(body: &[u8], content_type)` handles push responses:
     - JSON is parsed and walked;
     - `text/event-stream` walks each `data:` line (JSON when it parses, else text) with
       paths `events[i]`;
     - anything else is one item at `body`;
     - over 1 MiB is `TooLarge`.

3. **Normalise (`safety/normalize.rs`, §4 step 2):**
   - `Normalized { text: String, map: Vec<u32> }`. `map[i]` is the byte offset in the
     original of normalized byte `i`, so spans found in `text` map back for redaction.
   - The normalisation, one character at a time:
     - NFKC;
     - drop zero-width and bidi controls (U+200B–U+200F, U+202A–U+202E, U+2060–U+2064,
       U+FEFF, U+00AD);
     - fold confusables with a small table (Cyrillic and Greek letters that look Latin:
       `а е о р с у х і ј ѕ ԁ ӏ`, `ο α ν τ κ ι`; mathematical alphanumerics are covered by
       NFKC);
     - fold Unicode decimal digits to ASCII;
     - lowercase.
   - Per-character NFKC is enough for detection. Redaction never uses the normalized
     text.

4. **Decode (`safety/decode.rs`, §4 step 3):**
   - `decode_all(items) -> Vec<Decoded>`.
   - `Decoded { item: usize, range: Range<usize> (original bytes), layer: u8, encoding, text }`.
   - Finders, regex-based, on the original text:
     - base64 and base64url, 16+ characters, padding optional. Accepted when it decodes
       to valid UTF-8 that's at least 90% printable (letters, digits, punctuation,
       whitespace);
     - hex, 16+ digits, even length, same acceptance;
     - percent-encoded runs with at least two `%xx` sequences;
     - `\uXXXX` and `\xXX` escape runs;
     - HTML entities (named: the common 30; numeric: `&#…;` and `&#x…;`).
   - Decoded text is decoded again, up to `MAX_DECODE_DEPTH`.
   - Each span is capped at `MAX_DECODE_BYTES`; a longer span is left to opaque detection.
   - Decoded text is normalised (step 3) before finding candidates in it.

5. **Contact candidates (`safety/contacts.rs`, §4 step 4).** Run on each item's
   normalized text and each decoded text.
   - **Output:**
     - `Candidate { id, kind, source: Source::{Item{item, range}, Decoded{decoded}}, text, context }`;
     - `context` is up to `CONTEXT_BYTES` around the match;
     - `text` is capped at `CANDIDATE_BYTES`.
   - **Emails:** the usual form, plus spelled-out forms: `at` / `[at]` / `(at)` / `@` and
     `dot` / `[dot]` / `(dot)` / `.`, with any spacing.
   - **Phones:**
     - E.164 (`+` and 8–15 digits, with separators);
     - Indian mobiles (optional `+91` / `91` / `0`, then 10 digits starting 6–9, any
       grouping);
     - other 10–15 digit runs with phone-like separators;
     - digits written as English words, 7+ in a row ("nine eight four …", "double
       five").
     - **Excluded:** ISO dates and times, amounts with a currency or decimal point,
       version strings, UUIDs, and runs inside longer alphanumeric tokens.
   - **URLs:** `scheme://…` and bare domains. A bare domain needs a TLD on a fixed list
     (common generic TLDs and country codes, including `in`, `io`, `ai`, `me`, `co`), so
     `file.txt` and `Node.js` aren't matches.
   - **Messaging links:** a URL whose host is on a fixed list is reported as
     `messaging_link` instead of `url`. The list: `t.me`, `telegram.me`, `wa.me`,
     `chat.whatsapp.com`, `discord.gg`, `discord.com/invite`, `signal.me`,
     `linkedin.com/in`, `instagram.com`, `x.com`, `twitter.com`, `facebook.com`, `fb.me`,
     `snapchat.com/add`, `line.me` and `u.wechat.com`.
   - **Handles:** `@` plus 3–30 of `[a-z0-9_.]`, not part of an email.
   - **App names alone don't match** ("Telegram", "WhatsApp"); that's `off_platform`'s
     job in S2.
   - **Overlaps:** the longer match wins (an email over its domain).
   - **Numbers:** a walked number item is a `phone` candidate when its digits pass the
     phone rules.

6. **Opaque spans (`safety/opaque.rs`, §4 step 4):**
   - A span is opaque when all of these hold:
     - 64+ characters from a base64, base64url or hex alphabet;
     - Shannon entropy ≥ 3.5 bits per character;
     - not decoded to readable text in step 4.
   - Not opaque:
     - UUIDs;
     - hex of digest lengths (32, 40, 64 or 128);
     - JWTs (three base64url segments whose first two decode to JSON);
     - anything under a schema field declared binary.
   - `schema_at(schema, pointer)` follows `properties`, `items` and `prefixItems` (the
     subset capability schemas use) and returns the field's `format`, `contentEncoding`
     and `contentMediaType`. Contacts (S2's context) use `format` too.

7. **Language (`safety/language.rs`, §4 step 4):**
   - `Language { code: String, english: bool, reliable: bool }`.
   - **Input:** the items' text, joined with spaces, sampled to the first 4 KiB.
   - **Short text:** under 20 characters is `english: true, reliable: false`, so the
     English thresholds apply.
   - **Detection:** `whatlang::detect`.
     - English means `Lang::Eng` and `is_reliable()`.
     - Anything else (including an unreliable result) means not English.
   - **Hinglish:** Latin-script text with at least two distinct romanized-Hindi function
     words, making up at least 10% of the words, is `code: "hi-Latn"`, not English,
     whatever `whatlang` says.
     - The marker list: `hai hain nahi nahin kya kyun aap tum hum mein main mera meri tera
       kar karo karna raha rahi tha thi bhi aur toh ko ka ki se`.

8. **Budget and chunks (`safety/budget.rs`, §4.1):**
   - `Context`: the capability name and description, grant purpose, consent purpose,
     friendship messages, or a note's relationship context.
     - `Context::capped()` cuts each field to its §4.1 cap (description 2 KiB, each
       friendship message 1 KiB, purposes 500 characters), with a `"…[cut]"` marker.
   - `plan(channel, items, decoded, candidates, &context) -> Plan`:
     - **chunks:** `Vec<Value>` states, at most 4;
     - **judged:** the ids of the candidates judged in chunk 1, at most 16, in order;
     - **unjudged:** findings for text past the budget.
   - **How it fills:**
     - context and the first 16 candidates go into chunk 1;
     - fields are packed in item order. A string longer than the room left is split on a
       character boundary into `{path, part, text}` entries;
     - decoded text follows the chunk's fields, at most 4 KiB;
     - every addition is measured as **serialized** JSON (escaping included), against
       `STATE_BUDGET_BYTES`;
     - the next chunk opens when one is full;
     - what's left after chunk 4 becomes `unjudged` findings: one per remaining item path,
       action `Flagged`.
   - `halve(chunk) -> (Value, Vec<Entry>)` supports S2's 4xx retry: it splits a chunk's
     text in two and returns the second half.

9. **Redaction (`safety/redact.rs`, §4.3):**
   - `redact(value: &Value, items, decoded, candidates, ids: &[CandidateId]) -> Value`.
   - **Item candidates:** the original byte range is replaced by `[contact removed]`.
   - **Decoded candidates:** the whole encoded source range (layer 1's range in the
     original) is replaced.
   - **Several edits in one string** are applied from the end, so earlier ranges stay
     valid.
   - **A string whose trimmed text is entirely the candidate** becomes
     `"[contact removed]"`.
   - **A number item** becomes the string `"[contact removed]"`.
   - **A key** is renamed to `[contact removed 1]`, `[contact removed 2]`… The numbering
     is per object, skipping names already present.
   - The value is cloned and edited by pointer; the input is untouched.

10. **The labelled set (`backend/shared/tests/data/safety/messages.json`, §8):** about 150
    entries, each:
    ```json
    { "id": "…", "channel": "request|reply|note", "lang": "en|hi|bn|hi-Latn|es|zh",
      "capability": { "name": "…", "description": "…", "input_schema": { … } },
      "value": { … },
      "code": { "candidates": [{ "kind": "email", "text": "…" }], "opaque": 0,
                "english": true },
      "expect": "deliver|redact|block|flag" }
    ```
    - `code` is what S1's tests check.
    - `expect` is what S2's live eval checks against Jev.
    - Groups:
      - 30 benign, across languages;
      - 20 legitimate contact data (`book_meeting` attendees, `send_invoice` billing
        email, a support URL in a capability made for links);
      - 35 off-platform attempts: plain; obfuscated with words, spacing or look-alikes;
        Devanagari digits; base64 and hex wrapped; split across fields;
      - 25 harmful (violence, crime, harassment, self-harm, malware). These are short
        requests, never operational detail;
      - 15 opaque vs real tokens (ciphertext, a JWT, digests, UUIDs, a base64 image in a
        declared field);
      - 15 Hinglish, Hindi and Bengali variants of the above;
      - 10 adversarial (instructions aimed at the judge, padding).

11. **Tests (unit, in each file, plus `backend/shared/tests/safety_fixtures.rs`):**
    - **Walk:**
      - keys, strings and phone-shaped numbers walked; booleans and short numbers
        skipped;
      - exactly 1 MiB passes and 1 MiB + 1 is `TooLarge`;
      - event streams and plain bodies;
    - **Normalise:**
      - zero-width text inside an email;
      - Cyrillic look-alikes;
      - fullwidth letters and digits;
      - Devanagari and Bengali digits;
      - the offset map round-trips spans;
    - **Decode:**
      - each encoding;
      - base64 inside hex (two layers) found, a third layer not;
      - binary base64 rejected as unreadable;
      - the 64 KiB span cap;
    - **Contacts:**
      - each kind and each phone form;
      - spelled-out email;
      - English digit words;
      - the exclusions (dates, amounts, versions, UUIDs, `file.txt`, `Node.js`);
      - app names alone not matched;
      - overlaps;
    - **Opaque:**
      - random base64 and hex flagged;
      - JWT, UUID, digests and declared-binary fields not flagged;
    - **Language:**
      - each fixture language;
      - Hinglish markers (one marker isn't enough);
      - short text;
    - **Budget, the property the spec depends on:** for every fixture and for generated
      worst cases, every chunk's serialized state is ≤ 24 KiB. The worst cases:
      - 200 KiB of padding;
      - all control characters (6× escaping);
      - 4-byte characters and emoji ZWJ sequences;
      - a 100 KiB capability description;
      - 50 candidates.

      A 200 KiB message gives 4 chunks and `unjudged` findings with paths. Candidates past
      the 16th are listed as unjudged candidates.
    - **Redaction:**
      - in place, several per string;
      - an encoded span;
      - a whole string;
      - a number;
      - two colliding keys;
      - the input value unchanged.
    - **Fixtures:** every entry's `code` expectations hold (candidate kinds and texts, the
      opaque count, the English class).

12. **This PR also commits** the spec (already on the branch), with §8's fixture path
    updated, and this plan.

13. **Verify locally:**
    - `cargo test -p chakramcp-shared` and the whole workspace;
    - clippy;
    - a quick timing run (an `#[ignore]` test), noted in the PR body: walking, decoding
      and finding over a typical 2 KiB message and over 1 MiB.

## Open for S2's plan

- Moving `ComplianceChecker` and the questions from `relay/src/compliance` to
  `chakramcp_shared::system_one`, and building it in `app/src/main.rs` and
  `server/src/main.rs`.
- Question wording for `request.fields` / `reply.fields` when screening is on, and today's
  wording when it's off.
- `violations()` split by action, with language-aware thresholds.
- The `safety::screen` orchestration, which calls S1's pieces, runs the chunks in
  parallel, and applies the policy.
- Wiring into REST, MCP and A2A requests, A2A `trust_snapshot`, blocked-row markers, and
  the `code` field on rejected invokes.
- `SYSTEM_ONE_SAFETY`, metrics, the eval example, and Compose and `.env.example`.
