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
[`backend/relay/src/compliance/questions.rs`](../backend/relay/src/compliance/questions.rs),
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
saved in the invocation's `trust_snapshot.system_one`, which the audit
log returns. The report lists the model version, the probability for
each question, the threshold and the latency.

When a call is denied:

- REST (trusted or public) returns 403 with the usual body,
  `{ "invocation_id": …, "status": "rejected", "error": "System One compliance check denied this request: request is outside the grant's stated purpose (off_purpose=0.93)" }`,
  and writes a `rejected` invocation row with the report in its
  `trust_snapshot`.
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
messages all go to TypeSafe. Tell your users if that matters where you
run.

Treat the check as an extra layer on top of the grant checks, which stay
authoritative. TypeSafe's own docs say Jev reads instructions literally
and that adversarial input can move its answers, so the questions tell
it to treat quoted text in the input as data to judge.

Enforcement starts the moment the checks are on. If legitimate calls get
rejected, read `trust_snapshot.system_one.answers` on the rejected rows,
then make the grant's purpose more specific or adjust the wording and
`DENY_THRESHOLD` in `questions.rs`. The `off_relationship` question is
the most likely to misfire, because friendship messages are often just
greetings.

TypeSafe bills only for input tokens, currently about $0.042 per
million.
