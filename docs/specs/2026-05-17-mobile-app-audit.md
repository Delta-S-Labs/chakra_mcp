# `/app/*` mobile audit — 2026-05-17

Walk through every page under `frontend/src/app/(app)/app/` to catalog
mobile-breakage. Simulated viewports: 360px (iPhone SE), 414px (iPhone
14), 768px (iPad). Existing breakpoints in the codebase are 720px and
760px / 920px (shell only) — none of them target the actual phone
regime (≤480px), so this audit drives a 480px / 540px / 640px
breakpoint pass plus a few component-level fixes.

This audit drives a single follow-up commit on the same branch. CSS-
first, no component restructuring beyond minimal table-scroll wrappers
where missing.

## Shell (`layout.tsx`, `shell.module.css`, `AppNav.tsx`, `UserMenu.tsx`)

- **Header `flex-wrap: wrap` at ≤760px** dumps the `.nav` onto its
  own row but the nav itself uses `display: flex; gap: 0.4rem` with no
  overflow handling. At 360px it wraps awkwardly across two more rows
  (6 pills × ~75px = 450px total). Should become a horizontal-scroll
  strip below ~540px.
- **`.userTriggerMeta` is hidden ≤920px** but the avatar + caret chip
  still sits to the right and the brandmark on the left. OK at 414px,
  but the dropdown's `right: 0; min-width: 14rem (224px)` can clip
  off-screen-left on a 360px viewport when the chip is itself flush-
  right — needs a `max-width: min(320px, calc(100vw - 1rem))` clamp.
- **`userDropdown` has no max-height** — Account / API keys / Pair /
  Usage / Audit / Admin / Sign out can exceed the viewport on small
  phones with the URL bar showing. Should cap with `max-height` +
  `overflow-y: auto`.
- **Brandmark `.brandWord` is 0.84rem** — readable, but the dot+gap
  together waste ~40px when we need room for the trigger chip.

## CommandPalette (`commandPalette.module.css`)

- **`width: min(560px, 100%)`** is fine. **`padding: 14vh 1rem 1rem`**
  on the backdrop is fine.
- **`.list` is `overflow-y: auto` inside a `max-height: 70vh` panel**
  — works. No mobile changes needed here. (Only minor: chord keys
  next to label could wrap awkwardly at <360 but acceptable.)

## /app (dashboard)

- **`.statGrid` `minmax(200px, 1fr)`** at 440px yields 2-column cards
  about 195px wide each — cramped 2.4rem stat numbers + label + hint
  all squeezed. Should drop to single-col below ~480px and lower the
  floor to 160px so >480px can comfortably go 2-col.
- **`.activityTable` has no scroll wrapper** — 5 columns (When,
  Capability, From→To, Status, Elapsed). At 360px the table is forced
  smaller than its content; columns crush each other and `partyCell`
  spills. Wrap with `tableScroll` shared primitive.
- **`.welcome .title` uses `clamp(2.4rem, 5vw, 4rem)`** — at 360px
  that's 2.4rem (~38px). Acceptable. Slightly tighter on smallest
  phones would help horizontal margin pressure.
- **Page-level horizontal padding `2rem 1.25rem`** on `.page` wastes
  20px each side at 360px (= 40px of 360). Cut to 0.75rem ≤480px.

## /app/account

- Uses `orgs.module.css`. Affected by the same `.row` issues — see
  /app/orgs below.

## /app/admin

- **3 tables** (`Users`, `Organizations`, `API keys`) each have
  `tableWrap` already (`overflow-x: auto`). Good. But on a 360px
  viewport, horizontal scroll is non-obvious without a visual hint —
  add the fade-edge shared primitive `tableScroll` (drop-in
  replacement for `tableWrap`) for discoverability.
- **`.body` line `Visible only to the user whose email matches
  ADMIN_EMAIL on the backend`** — the long `<code>` element fits, OK.
- **`.empty` cell uses padding 1.5rem** — fine.

## /app/agents (list + create form)

- **`CreateAgentForm` `.fields` is `minmax(0,1fr) minmax(0,1fr)
  auto`** — collapses to 1-col at 720px (good). No regression to add.
- **`.row` uses `flex; justify-content: space-between`** — `Open →`
  link sits to the right. At narrow widths the `rowMeta` (slug +
  capability count) can be long with org slugs; needs to wrap below
  the name. The flex doesn't wrap → text overflows. Add a 540px
  breakpoint stacking name above the Open link.
- **`<code>{a.account_slug}/{a.slug}</code>`** strings can be long
  (org slug + agent slug). Needs `overflow-wrap: anywhere` so they
  don't push the row off-screen.

## /app/agents/[id] (edit + capabilities)

- **`EditAgentForm.tsx`** uses `.fields` from agents.module.css —
  same as create. Collapses to 1-col at 720px. Good.
- **`CapabilitiesPanel.tsx`**:
  - `.capFields` is `grid-template-columns: 1fr 1fr auto` collapsing
    to 1-col ≤720px. Good.
  - **Name+Visibility row remains 2-col at 540-720**: at 540 the
    select crushes against the input. Should collapse to 1-col
    earlier (≤640).
  - The `<button type="submit" className={styles.create}>` sits as
    the third column at desktop. At 720→1col, it lands as the last
    full-width child. Acceptable.
  - **JSON `<textarea>` elements** for input/output schemas are
    `font-family: var(--font-mono); font-size: 0.85rem` — at 360px
    they can render their default 20-char wide content fine because
    `width: 100%`. Good.
  - **Empty card `<code className={styles.emptyCmd}>` long CLI
    snippet** has `word-break: break-word` already. Good.
- **Cap row `.capRow` flex** — same fix as agent `.row`: stack at
  ≤540.

## /app/agents/network

- **Same `.row` issue** as `/app/agents` list.

## /app/api-keys (list + create form)

- **Create form `.form` is `2fr 1fr auto`** — already collapses to
  1-col at 720. Good.
- **`.row` (api-keys list)** already collapses to stretch at 720.
  Good.
- **`.created` two-column reveal panel** for the just-created key:
  long plaintext token (~40 chars) inside `.createdValue` with
  `word-break: break-all`. Good. The two-column layout with `Copy`
  button to the right is fine, but on phones the Copy button gets a
  thin label area. Stack to 1-col below 540.
- **Tap targets**: `.revoke` and `.rotate` buttons are
  `padding: 0.4rem 0.8rem; font-size: 0.66rem` — physically ~28×24
  px. Below iOS 44px minimum. Bump to >=40px height on mobile.

## /app/api-keys/[id] (usage charts)

- **`.actionBar` flex-wrap** — Rotate + Revoke buttons. Good.
- **`.chartGrid` `minmax(280px, 1fr)`** — collapses cleanly to 1-col
  on phones. Good.
- **`.byAgentTable` no scroll wrapper** — 3 cols (Agent, Capability,
  Count). Slug + capability_name can be long; add `tableScroll`.
- **`.created` reveal panel** same fix as /app/api-keys list.

## /app/audit

- **`.filters` with picker + tabs** — `flex-wrap: wrap` and good.
- **`.agentSelect` has `min-width: 16rem (256px)`** — on a 360px
  viewport with `padding: 0 0.4rem` on `.filters`, the select can
  hit 256+ and force horizontal scroll on the whole page. Lower min-
  width to `min(16rem, 100%)` or `auto` at mobile.
- **`.pre` (request/response JSON viewer)** has `overflow-x: auto;
  white-space: pre`. Good.
- **`.rowHeader` clickable row** — `padding: 0.85rem 1rem`, large
  hit area. Good (44+ px tall).

## /app/friendships

- **`.fields` `minmax(0,1fr) minmax(0,1fr) auto`** — collapses to
  1-col at 720. Good.
- **`.row` `grid-template-columns: 1fr auto`** — collapses to 1-col
  at 720. Good.
- **Long messages in `.quote`** — readable; wraps naturally.
- **Tap targets**: `.create`, `.secondaryBtn`, `.dangerBtn` are
  `padding: 0.55rem 0.95rem; font-size: 0.74rem` — ~34px tall.
  Below iOS 44px minimum but acceptable for now; bump touch padding
  on mobile.

## /app/grants

- **`.fields` 4-col `1fr 1fr 1fr auto`** — collapses to 1-col at
  720. Good.
- **`.invokeResult` `<pre>` JSON output** has `overflow-x: auto;
  white-space: pre`. Good.
- **`.row` collapses at 720**. Good.
- **Long capability names in `.capCode`** — fine, they're short.

## /app/inbox

- **`.controls` is `1fr auto`** — collapses to 1-col at 720. Good.
- **`.respondTabs` is `1fr 1fr`** — collapses to 1-col at 720.
  Good.
- **`.pre` (request/result JSON)** — `overflow-x: auto`. Good.
- **Long meta lines (`from <agent> · capability · received ...`)**
  use `.rowMeta` with `flex-wrap: wrap; gap: 0.45rem`. Good.

## /app/orgs (list)

- **`.cardList` `minmax(min(100%, 380px), 1fr)`** — at 360px the
  `min(100%, 380px)` clamps to viewport width, so 1-col. Good.
- **Inside each card, `.agentRow` is flex with wrap at 720**. Good.
- **Long `<code>{slug}/{slug}</code>` strings in `.agentSlug`** —
  add `overflow-wrap: anywhere`.

## /app/orgs/[slug]

- **Agents list `.row` with `.rowActions` (MoveAgentButton + Open
  link)** — flex, no wrap behaviour at ≤540. The two pill-buttons
  push the row content tight. Should stack at ≤540.
- **Members list `.row`** — avatar + name + email + role badge.
  Long emails wrap fine because `.rowMeta code` uses default
  break (good); but `.row` itself flex-wraps awkwardly.
- **`InviteForm`** `.inviteForm` is `2fr 1fr auto` — collapses at
  720. Good.
- **`.invite` (just-generated link)** is `1fr auto` with
  `.inviteUrl` long: `word-break: break-all`. Good.
- **`DeleteOrgButton` in `.dangerZone` `.dangerRow` is flex with
  `flex-wrap: wrap`**. Good.
- **MoveAgent / DeleteOrg `Modal`s** — width is `min(440px, 100%)`
  with `padding: 1rem` backdrop. Fits 360px viewport (440 vs 360+pad
  → clamps to 100% of inner = 360-32 = 328px). Good.

## /app/pair

- **`.codeCard` is `width: min(540px, 100%)`** centered. Good.
- **`.codeInput` `font-size: 1.45rem; letter-spacing: 0.18em`** — at
  360px, an 8-char code like "ABCD-1234" is ~290px wide. Fits.
  Acceptable.
- **`.facts` `<dl>` is `grid-template-columns: max-content 1fr`** —
  works at narrow widths.
- **`.pairRow` flex with wrap at 720, then column-stack**. Good.
- **`.pairHintBody` `width: min(340px, 80vw)`** — fits.

## /app/usage

- **`.rangeBar` is `display: inline-flex; flex-wrap: wrap`**. Good.
- **`.rollupTable` (used 4× for by_org / by_agent / by_api_key /
  by_pair) has no scroll wrapper**. Two columns (label + numericCol).
  Labels can be very long (slug + display name). Wrap with
  `tableScroll`.
- **`.totalCard .chartFrame min-height: 180px`** — fine.
- **`.actionHeader` is flex-wrap**. Good.

## /app/welcome

- **`.option` `grid-template-columns: auto 1fr`** — `<input
  type="radio">` + label/hint. Works at narrow widths.
- **`.chips` flex-wrap**. Good.
- **`.actions` `justify-content: flex-end`** with submit button —
  on mobile the button should be full-width for thumb-reachability.
  Stack + stretch below 540.

---

## Shared primitives to add

1. **`.tableScroll`** wrapper class (in `shell.module.css`):
   `overflow-x: auto; -webkit-overflow-scrolling: touch;
   border-radius: 0.85rem; position: relative;` + a right-edge
   gradient fade ::after so users know there's more.
   Drop-in for `audit/page.tsx` (recent activity), `usage/UsageView`
   (4 rollup tables), `dashboard/page.tsx` (activity table),
   `admin/page.tsx` (3 tables — replaces `tableWrap`),
   `api-keys/[id]/UsageCharts` (by-agent table).

2. **`.idCode`** utility (in `shell.module.css`): wraps
   `<code>UUID/slug</code>` strings with `overflow-wrap: anywhere`
   so they never burst rows. Applied to agent-row meta.

3. **`.appPage`** wrapper utility (in `shell.module.css`):
   `min-width: 0` on grid children so long content doesn't push
   parent grids wider than viewport.

## Phone breakpoints introduced

- **`max-width: 540px`** — single-col list rows, stack form button
  full-width.
- **`max-width: 640px`** — earlier collapse for cap-form Name+
  Visibility pair.
- **`max-width: 480px`** — single-col stat grid, tighten page padding,
  shell nav becomes horizontal-scroll strip.

## Out of scope (deliberately deferred)

- Hamburger / sheet-nav rewrite (per spec).
- Adding tap-target padding to every button site-wide — done piecemeal
  where worst (api-key revoke/rotate).
- Replacing recharts with a hand-rolled SVG on narrow widths.
- Pull-to-refresh / native-feel touches.
