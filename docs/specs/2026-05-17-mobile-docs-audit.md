# /docs/* mobile-friendliness audit

**Status:** audit (2026-05-17)
**Branch:** `feat/docs-mobile-friendly-pass`
**Sister branch (separate scope):** `feat/app-mobile-friendly-pass` (covers `/app/*`)

## Scope

Four routes under `frontend/src/app/(site)/docs/`:

- `/docs` — `page.tsx` (199 lines)
- `/docs/concepts` — `concepts/page.tsx` (252 lines)
- `/docs/quickstart` — `quickstart/page.tsx` (228 lines)
- `/docs/agents` — `agents/page.tsx` (1,064 lines — template-heavy)

Plus the single shared module `frontend/src/app/(site)/docs/docs.module.css` and the global `frontend/src/styles/site.css`.

Reference viewport: 360x780 (small Android, the worst-case real-world device).

## Findings at the shared-module level (`docs.module.css`)

These hit every page because every page imports this module.

- **`.shell` padding is `4rem 1.5rem 6rem`** — that horizontal `1.5rem` is fine on phone, but the top `4rem` plus the marketing topbar plus large `.title` immediately eats the fold. Needs a narrow-width override to ~`2.4rem 1rem 4rem`.
- **`.title` uses `clamp(2rem, 4.5vw, 3.4rem)`** — at 360px viewport that resolves to `2rem` (32px), which is fine. The headlines on each page (a four-word phrase) wrap to two lines and don't bust. Leave as is.
- **`.pre` has `overflow-x: auto`** but **no `-webkit-overflow-scrolling: touch`** (matters for iOS momentum), **no right-edge fade**, and **`padding: 1rem 1.1rem`** — that 1.1rem right padding is too tight for the last char on long shell lines. The block itself works; the affordances don't.
- **`.shell code` (inline) has no `overflow-wrap`** — long identifiers like `verification_uri_complete`, `urn:ietf:params:oauth:grant-type:device_code`, `/agents/<account>/<slug>/.well-known/agent-card.json`, and `examples/hermes-openclaw-demo` bust the 360px container and create horizontal page scroll. This is the single most disruptive issue.
- **`.cardGrid`** already uses `repeat(auto-fit, minmax(min(260px, 100%), 1fr))` — collapses to single column correctly on phone. No action needed.
- **`.callout` padding `0.85rem 1rem` and `border-radius: 0 0.5rem 0.5rem 0`** — fine.
- **TOC sidebar (`.toc`, `.tocList`) only activates at `min-width: 960px`** via media query — hidden on phone by default. Good. No content is being eaten on narrow widths.
- **`.langTabs`** already wraps via `flex-wrap: wrap`. Good.
- **Tap targets**: `.card`, `.langTab`, anchor links — the `.card` is plenty large (padding 1.2rem 1.3rem). `.langTab` at `padding: 0.35rem 0.75rem` + font 0.7rem comes in under 44px tall; not currently rendered anywhere in the four pages I audited (defined but unused), so cosmetic, but worth flagging if it becomes used.

## /docs (index)

- **No code blocks**, **no tables**. Pure prose + two card grids. The card grid is fine via `auto-fit`.
- Heavy use of inline `<code>`, including:
  - `/agents/<account>/<slug>/.well-known/agent-card.json` — long, busts container on 360px.
  - `https://chakramcp.com/.well-known/chakramcp.json` (anchor text, not in code, fine).
  - `brew tap Delta-S-Labs/chakra_mcp && brew install chakramcp-server` (inside a card body) — body text wraps, but card looks ugly with the unbroken token.
- **`<a>`** inline links to long GH URLs in `<li>` lists — long URLs are link text and don't wrap; the anchor underline keeps them on one line.
- Tap targets fine.

## /docs/concepts

- **One `<pre>` block** at line 215 — a one-liner (`chakra.inbox.serve(agentId, handler)`). Fits easily.
- Many inline `<code>` lifecycle states (`proposed`, `accepted`, `rejected`, …) — short, all fine.
- Long inline `<code>`:
  - `/agents/<account>/<slug>/.well-known/agent-card.json` — busts.
  - `POST /agents/<…>/a2a/jsonrpc` — fits.
  - `counter_of_id` — fits.
- No tables, no side-by-side grids beyond the inherited TOC.
- No specific page-level issues beyond the shared `<code>` overflow.

## /docs/quickstart

- **9 `<pre>` blocks** — every one is a shell command or a TS snippet, all wide:
  - `cargo install --git https://github.com/Delta-S-Labs/chakra_mcp --branch main chakramcp-cli` (multi-line with backslash continuation)
  - `chakramcp friendships propose --from <my-agent-id> --to <their-agent-id> --message "Let's connect."` (multi-line)
  - The 7-line `git clone` block in the demo callout
  - The TS `inbox.serve` block
- Existing `.pre` has `overflow-x: auto` so they technically scroll, but the lack of touch momentum + no fade + tight right padding hurts.
- **Embedded `<video>`** — already has `width: 100%` inline style. Good. Should also set `max-width: 100%` to be safe, but `width: 100%` is fine.
- **Inline `<code>` offenders:** `~/.chakramcp/config.toml`, `chakramcp.com/app/api-keys`, the long `cargo install --git ...` URL when referenced inline, `chakramcp networks use <name>`. All bust on 360px.
- Tap targets fine.

## /docs/agents

The big one. 1,064 lines, ~40 `<pre>` blocks, ~150 inline `<code>` tags, six `<h4>` "TypeScript / Python / Rust / Go" sub-headings under section h3s.

- **`<main className={`${styles.shell} ${styles.wide}`}>`** — uses `.wide` (max-width 92ch). At narrow widths there's no max-width effect; just inherits `.shell` padding. Same overrides needed.
- **`<pre>` blocks here are the worst offenders.** Many are JSON examples spanning 10–20 lines. They scroll horizontally today (because `.pre` has `overflow-x: auto`) but:
  - Some, like the `message_owner` JSON schema (lines 882–908), have long URLs and nested structures — fine to scroll.
  - The bash polling loop (lines 271–305) is the widest single block — multi-line shell with `\` continuations. Scrolls.
  - The curl `device_authorization` example (lines 272–286) — same. Wraps lines but scrolls horizontally where individual lines are wide.
- **`<code>` inside `<p>`:** dozens. The bad ones:
  - `verification_uri_complete`, `verification_uri_qr`, `verification_uri` — appear in 6+ paragraphs.
  - `urn:ietf:params:oauth:grant-type:device_code` — explicit grant-type identifier.
  - `chakramcp.com/app/pair?session=ABCD-1234`.
  - `/agents/<account>/<slug>/.well-known/agent-card.json`.
  - `chakramcp capabilities add --template message_owner --agent <id>` (line 938 — appears inline in a `<pre>` so fine).
  - `chakra.inbox.serve_with_handlers` — fits.
  - `Authorization: Bearer <jwt>` — fits.
- **CLI ops `<ul>` (lines 357–411)** — each list item is a long compound `<code>...</code> — description` block. Some `<code>` strings here run 50+ chars; they bust. The shell-out command examples (`chakramcp capabilities add --template message_owner --agent <id>`) are the worst offenders.
- **Card grid at lines 120–145** — uses `.cardGrid`, collapses correctly.
- **Two callouts with very long copy** — fine, prose wraps.
- **The "Machine-readable shortcuts" callout** has long URL anchors that don't wrap — same `<code>` issue.
- **The `<h4 className={styles.h3}>`** sub-headings ("TypeScript", "Python", etc.) reuse `.h3` styling. Fine at narrow widths; just stacked one after another.

## Things noticed in the docs content itself (flag-only, don't fix here)

These are content/copy bugs unrelated to mobile:

1. **`/docs/quickstart`** line 38 uses `style={{ width: "100%", borderRadius: "12px", display: "block" }}` for the demo video — inline style in a Next.js component. Cosmetic, but inconsistent with the rest of the file which uses CSS module classes. Could become a `.demoVideo` class.
2. **`/docs/agents`** line 59 `<p style={{ marginTop: "0.5em", fontSize: "0.92em" }}>` — same inline-style smell.
3. **`/docs/agents`** "CLI capability commands are queued" (line 156) is a TODO masquerading as docs. Worth a status check — the CLI may now expose `capabilities add` (line 938 implies it does).
4. **`/docs/agents`** mentions `inbox.serve_with_handlers` (line 949) but `/docs/quickstart` and `/docs/concepts` only mention `inbox.serve` — possible inconsistency in surface area.
5. **`/docs/quickstart`** line 92 has a stray space in the link text: `<code>crates.io</code> ` — minor.
6. **`/docs/agents`** line 314 — `<a href="#sdk-pair">SDK § pair()</a>` — § character may need escaping check; renders fine in practice but worth checking.
7. **Anchor IDs collide:** `/docs/agents` uses `id="capabilities"` on an h3 (line 647) but `/docs/concepts` uses `id="capabilities"` on an h2 (line 96). They're on different routes so they don't collide in the browser, but it's a minor cross-page inconsistency for anyone deep-linking.

None of the above are touched by this PR.

## Per-page issue count

| Page | Issues |
| --- | --- |
| /docs | 2 (long inline-code overflow, narrow-width padding) |
| /docs/concepts | 2 (long inline-code overflow, padding) |
| /docs/quickstart | 4 (9 wide pre blocks need scroll affordances, long inline-code, padding, video class extraction — flagged not fixed) |
| /docs/agents | 6 (≥40 wide pre blocks, long inline-code throughout, narrow-width padding, CLI-ops list code overflow, callout URL overflow, .wide max-width has no effect on phone — fine) |

**Shared issues affecting all four:** 4 (inline `<code>` overflow, `<pre>` lacks touch + fade + right padding, `.shell` padding tight, no narrow-width override).

**Total distinct issues:** 14 (8 page-specific, 4 shared, plus the 6 content flags above which are out-of-scope).

## Fix plan (Phase 2)

In `docs.module.css`:

1. **New shared primitive `.codeScroll`** — wraps every `<pre>`. `overflow-x: auto`, `-webkit-overflow-scrolling: touch`, right-edge fade via mask-image gradient. Adopted on every page.
2. **`.pre` adjustments** — bump `padding-right` so the last char doesn't kiss the edge.
3. **Extend `.shell code`** — `overflow-wrap: anywhere` and `word-break: break-word` so long identifiers fold.
4. **Add `@media (max-width: 720px)` block** for `.shell`:
   - `padding: 2.4rem 1rem 4rem`
   - `.title` already has clamp — leave alone.
   - `.h2` slight scale-down from `1.55rem` to `1.4rem` for breathing room.
   - `.pre` font-size from `0.84rem` → `0.78rem` so more shell text fits without scroll.
5. **No JS changes.** No restructuring of the four `page.tsx` files except wrapping `<pre>` elements in the new `.codeScroll` div. That wrapper change touches every `<pre>` in the four pages.

No `/app/*` files touched. No new dependencies. No visual change above 720px.
