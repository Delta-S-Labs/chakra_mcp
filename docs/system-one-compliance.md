# System One compliance checks

Friendships and grants decide **who** may call **which** capability. They
say nothing about **what** is sent. An agent with a valid grant for
`propose_slots` can still put anything in the request input: a request
for something the capability was never meant to do, a prompt injection
aimed at the receiving agent, or an attempt to pull out credentials.

System One compliance checks close that gap. When they're turned on, the
relay reads each invocation's input and judges it against the capability,
the grant's stated purpose and the friendship before the call is queued or
forwarded. The judging is done by [TypeSafe](https://typesafe.ai)'s
System One model, **Jev**. Jev returns calibrated yes/no probabilities
rather than generated text, typically in 70–500 ms.

The feature is **off by default** and controlled only by environment
variables. There is no admin-panel switch.

## Turning it on

Set these where the relay runs: `.env` / `.env.local` in dev, and the
host env for `infra/docker-compose.prod.yml`. Restart the relay (or
`chakramcp-server`) to apply them.

| Env var                  | Default                   | Meaning                                                         |
|--------------------------|---------------------------|-----------------------------------------------------------------|
| `SYSTEM_ONE_CHECKS`      | `false`                   | `true` / `1` / `yes` / `on` enables the checks.                 |
| `TYPESAFE_AI_KEY`        | *(unset)*                 | TypeSafe API key ([console](https://console.typesafe.ai/keys)). Required when enabled. |
| `TYPESAFE_AI_MODEL`      | `jev-latest`              | Model id. Pin a version (e.g. `jev-1.13.0`) for stable behaviour. |
| `TYPESAFE_AI_BASE_URL`   | `https://api.typesafe.ai` | API base URL. Override only for a proxy or gateway.             |
| `TYPESAFE_AI_TIMEOUT_MS` | `2000`                    | Timeout for each attempt at the TypeSafe call.                  |

```bash
SYSTEM_ONE_CHECKS=true
TYPESAFE_AI_KEY=ts_...
TYPESAFE_AI_MODEL=jev-latest
```

At startup the relay logs `System One compliance checks enabled` with the
model and base URL. If `SYSTEM_ONE_CHECKS` is on but `TYPESAFE_AI_KEY` is
empty, the relay still boots. It logs an error and runs with the checks
**disabled**.

## What gets checked

The check runs on every invocation surface:

| Surface                         | Judged against                                        |
|---------------------------------|-------------------------------------------------------|
| REST `POST /v1/invoke` (grant)  | capability, grant purpose, friendship messages        |
| MCP `invoke` tool               | same as REST (one shared code path)                   |
| A2A `SendMessage`               | capability, grant purpose, friendship messages        |
| REST `POST /v1/invoke` (public) | capability only (the public tier has no grant or friendship) |

It runs **after** all the deterministic gates: grant exists and is active,
accepted friendship, account membership, and rate and quota limits. A call
those gates would refuse never costs a model call, and the check can only
**deny** a call they allowed. It never lets through a call they refused.

For each call the relay sends one request with up to five yes/no
questions. A question is asked only when there is evidence to judge it
against:

| Question id         | Asked when                                  | "Yes" means                                                  |
|---------------------|---------------------------------------------|--------------------------------------------------------------|
| `off_capability`    | the capability has a description            | the input asks for a clearly different task or data          |
| `off_purpose`       | the grant has a `purpose`                   | the input is clearly outside the stated purpose              |
| `off_relationship`  | the friendship messages are non-blank       | the messages set a scope, and the input clearly exceeds it   |
| `prompt_injection`  | always                                      | the input tries to override or redirect the receiving agent  |
| `data_exfiltration` | always                                      | the input seeks credentials, system prompts, or other people's private data |

Any answer at or above **0.85** denies the call. The questions and this
threshold are all in
[`backend/relay/src/compliance/questions.rs`](../backend/relay/src/compliance/questions.rs),
so review and tuning happen in one place.

Inputs larger than 24 KiB of JSON are cut before sending. The model sees
the leading 24 KiB plus a truncation marker.

### Grant purpose

Grants carry an optional `purpose`: free text of up to 500 characters
saying why the grantee needs access. Set it in the web UI (Grants →
Purpose), with `chakramcp grants create --purpose "…"`, with the
`purpose` field on `POST /v1/grants` or the MCP `create_grant` tool, or
through the SDKs' `CreateGrantRequest`. A specific purpose ("schedule the
weekly team sync") gives the check far more to work with than the
capability description alone. The purpose is also delivered to the
serving agent in the inbox `grant_context`.

## Outcomes

**Allowed.** The call proceeds as normal. The check's report goes into the
invocation's `trust_snapshot.system_one`, which the audit log returns. The
report holds the model version, each question's probability, the
threshold and the latency.

**Denied.**

- REST (trusted or public) returns **403** with the usual body shape:
  `{ "invocation_id": …, "status": "rejected", "error": "System One compliance check denied this request: request is outside the grant's stated purpose (off_purpose=0.93)" }`.
  A `rejected` invocation row is written, with the report in its
  `trust_snapshot`.
- MCP returns a tool error with the same message.
- A2A returns HTTP **403** with JSON-RPC error `-32009`,
  `data.code = "chk.policy.compliance_denied"` and the message in
  `data.detail`. Nothing is parked or forwarded. As with the other A2A
  policy denials, no invocation row is written; the denial is logged.

**TypeSafe unavailable.** Timeouts, connection errors, non-2xx responses
and malformed bodies all **fail open**. The call proceeds, a warning is
logged, and `trust_snapshot.system_one.decision = "error"` records what
happened. A single 429 or 529 (TypeSafe's back-off signals) gets one retry
after 200 ms. The relay's availability never depends on the provider's.

## Operating notes

- **Latency.** Each checked call pays one TypeSafe round trip, typically
  70–500 ms and capped by `TYPESAFE_AI_TIMEOUT_MS`.
- **Data leaves your deployment.** Invocation inputs (up to 24 KiB each),
  capability descriptions, grant purposes and friendship messages are sent
  to TypeSafe. Tell your users if that matters for your deployment.
- **This check adds a layer; it doesn't replace anything.** TypeSafe notes
  that Jev reads instructions literally and that adversarial state can move
  its answers. The deterministic gates stay authoritative, and the
  questions spell out that quoted content is data to judge, not
  instructions to follow.
- **Tuning.** It enforces from the moment it's switched on. If legitimate
  calls are being rejected, look at `trust_snapshot.system_one.answers` on
  the rejected rows. Then either sharpen the grant's purpose, or adjust the
  question text or `DENY_THRESHOLD` in `questions.rs`.
- **Cost.** TypeSafe bills input tokens only, currently about
  $0.042 per million.
