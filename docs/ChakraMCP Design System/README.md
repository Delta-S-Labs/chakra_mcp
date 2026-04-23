# ChakraMCP — Design System

> **ChakraMCP** is the relay network where AI agents discover each other, negotiate trust, and execute capabilities — with public menus, private friendships, and consent‑aware access control. A token economy underneath pays creators and lets users earn access by watching ads or renting idle device compute. It's protocol infrastructure with consumer energy.

This design system captures the brand's visual + verbal DNA so future surfaces (site, app, docs, decks, marketing) feel unmistakably like ChakraMCP — warm, editorial, a little rogue.

---

## Sources

- **Codebase**: `kaustav1996/agent_telepathy` on GitHub (imported to `app/`). Note: the codebase ships under an earlier working name "Agent Telepathy" — the brand is being renamed to **ChakraMCP**. All language, visuals, and structure come from this repo; only the wordmark is refreshed.
  - Design context file: `app/.impeccable.md`
  - Specs: `docs/superpowers/specs/2026-04-09-agent-network-design.md`, `.../2026-04-10-developer-page-redesign.md`
  - Core CSS: `app/src/index.css`, `app/src/App.css`
  - Copy: `app/src/content/concept.ts`, `app/src/content/developer.ts`
- **Figma**: none provided.
- **Brand notes**: see the "Additional notes" brief attached to this project — editorial / printed / warm coral + lime + butter, Archivo Expanded + Afacad, Phosphor icons, no blue, no SaaS dashboard energy.

---

## Index (root manifest)

```
README.md                  — this file
SKILL.md                   — agent-skill frontmatter for Claude Code
colors_and_type.css        — all design tokens (CSS vars)
fonts/                     — font files (currently Google Fonts @import; see CAVEATS)
assets/                    — logos, favicons, illustrations
preview/                   — Design System tab cards (typography, color, spacing, components, brand)
ui_kits/
  website/                 — marketing site recreation (portfolio + concept pages)
    index.html
    *.jsx
  app/                     — in-app surfaces (agent directory, inbox, proposal review)
    index.html
    *.jsx
app/                       — imported reference codebase (read-only context)
docs/                      — imported design specs (read-only context)
```

---

## CONTENT FUNDAMENTALS

**Tone**: mischievous, lucid, unpolished-on-purpose. Rogue, sly, slightly sarcastic — but still legible to a normal human who isn't steeped in protocol jargon. The product is protocol infrastructure, but the voice is an independent zine.

**Voice rules**:

- **Second person, familiar.** "You expose a public menu." "You can see who is playing." Never "users" or "the end-user."
- **Short declarative sentences with an attitude.** "Friendship is paperwork, not magic." "The relay is the bouncer." "The network does not trust vibes."
- **Rhythm via fragments.** A three-beat list is common: "public menus, private friendships, consent-aware access." Sentences often land on a concrete, slightly funny noun.
- **Metaphors grounded in physical places.** Venues, bouncers, menus, windows, paperwork, backstage, vending machine. Never "ecosystem," "platform," "solution," "empower."
- **Anti-corporate.** Call out tropes by name: "No 'DM me for details.'" "Not LinkedIn for bots." "The network does not trust vibes."
- **Headlines are full sentences with a period.** "Give agents a public menu, a private guest list, and a bouncer." Not fragments, not titles — statements.
- **Eyebrows / labels are uppercase display, 0.12em tracking.** Lowercase tags use display font, negative tracking.
- **No emoji.** Not in headlines, body, UI, nowhere. The brand uses typographic and color emphasis instead.
- **Case**: sentence case for headlines (`Give agents a public menu…`), Title Case never, UPPERCASE only for display-font eyebrows/tags with wide tracking.

**Examples (verbatim from the codebase)**:

> "Give agents a public menu, a private guest list, and a bouncer."
>
> "Discovery is public. Access is negotiated. Consent can be per run. The relay checks the paperwork every single time."
>
> "Not LinkedIn for bots."
>
> "Friendship is paperwork, not magic."
>
> "The relay is the bouncer."
>
> "You expose a public menu, a friend menu, and the uncomfortable stuff that still needs a human or admin to say yes. The point is not openness at any cost. The point is controlled usefulness."

**Never say**: "empower," "leverage," "unlock potential," "best-in-class," "seamless," "cutting-edge," "revolutionary," "game-changer," "AI-powered" (as a bare adjective).

---

## VISUAL FOUNDATIONS

### Colors

Warm cream backgrounds, dark brown-black ink, **three accents**:

- **Coral** `oklch(63% 0.19 28)` — terra cotta. Primary action, coral dots, primary pills, hero punctuation. Never blue, never red, never orange.
- **Lime** `oklch(86% 0.16 128)` — unexpected chartreuse. Secondary tints, soft card backgrounds, "go" states.
- **Butter** `oklch(91% 0.13 96)` — pale yellow. Badges, step markers, highlights, the rotated "note badge" tag.

Text is `oklch(24% 0.03 18)` — dark brown-black, never pure black. Lines / dividers are warm `oklch(78% 0.02 38)`. **No blue** anywhere in the palette. **No purple-to-blue gradients.** **No neon.**

### Type

- **Display**: Archivo Expanded, 600/700/800. Negative tracking `-0.03em` to `-0.05em`, line-height 0.92–1.02. Used for headlines, eyebrows (uppercase, 0.12em tracking), nav links, tags, step markers, badges.
- **Body**: Afacad 400/500. Normal tracking, 1.6 line-height, max-width 68ch.
- **Mono**: system stack `ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas`. Used for endpoints, tokens, event types, JSON payloads.

Headlines are **typeset-feeling**, not rendered: tight, wide, heavy, stamped. The expanded width of Archivo is load-bearing — don't substitute a normal-width geometric sans.

### Backgrounds

Default body bg is a subtle two-stop radial: butter at top-left, coral at top-right, fading into cream. Over it, a **faint grid** (88px × 88px hairlines) fades from 18% opacity at top to 0% at 82% down the page. This is the "paper with ruling" feel. Never full-bleed photography. Never dark mode.

### Cards

- Radius: `1.4rem`–`1.6rem` (large, confident).
- Border: 1px hairline in `--line` color, with a small ink-mix for a slight crisp edge.
- Background: always a vertical linear-gradient between two warm papers — never flat white.
- Shadow: `0 28px 50px` with 10% ink alpha — soft, far-thrown, warm.
- Decorative sploosh: key hero cards have a blurred butter or coral circle at the bottom-right corner (`::after`), 45% opacity, for a printed-risograph feel.

### Pills, badges, chips

- Pills (nav / buttons): pill-radius, 1px border, `--paper-soft` bg, display-font 0.72rem uppercase 0.08em tracking.
- **Primary pill** = coral fill, cream text, coral-darkened border.
- **Note badge** = butter fill, rotated **-3deg**, display font, small drop-shadow. Signature "zine tag" flourish.
- Tags = pill-radius, warm-white, 0.72rem uppercase.

### Borders & hairlines

1px everywhere for divider lines. Dashed coral borders on "aside" / boundary panels. Decorative stripes use `color-mix` for warmth, not pure gray.

### Shadows

Two systems: **soft + warm** (`--shadow-soft` / `--shadow-md`) for every surface, and **butter glow** (`--shadow-butter`) for rotated note-badges. No hard drop shadows. No inner shadows.

### Layout

- Content max-width `1180px`, gutters `1.5rem`.
- Hero blocks are asymmetric two-column (`1.05fr 0.95fr` or `1.08fr 0.92fr`).
- **Staggered tiles**: highlight grids offset every other tile by `1.1rem` vertically — gives the page a handset-type look.
- Sticky header sits `0.75rem` from the top, rounded pill container.

### Motion

- Easing: `cubic-bezier(0.25, 1, 0.5, 1)` — a confident ease-out-quart.
- Page reveals: `translateY(24px)` → `0` + opacity fade, 700ms, with 110ms / 180ms stagger.
- Interactive hovers: `translateY(-1px)` on pills, fast (`180ms`). No scale, no rotation on hover.
- Press: no explicit state — relies on instant transform settle.
- `prefers-reduced-motion`: all animations clamped to 0.01ms.

### Hover / press states

- **Pills / nav links**: translateY(-1px), border darkens subtly (color-mix with ink).
- **Active nav**: butter fill, coral-tinted border.
- No opacity-change hovers. No color-swap hovers on buttons.

### Transparency & blur

- Sticky header uses `backdrop-filter: blur(14px)` over a translucent paper-mix.
- Concept rail uses `backdrop-filter: blur(10px)` — same trick, lighter mix.
- Otherwise: no frosted glass, no translucent cards.

### Iconography & imagery

- **No bitmap imagery** in the codebase. The brand is typography + color + printed flourishes.
- Icons are **Phosphor Regular** (see `ICONOGRAPHY` below) when needed — currently the codebase uses mostly pseudo-element shapes (coral dot, circle-behind-badge).
- **Placeholder rule**: if an icon is missing, use a solid geometric shape (dot, square, ring) in an accent color rather than inventing a lucide/heroicon look-alike.

### Corner radii summary

| Token         | Value       | Use |
|---------------|-------------|-----|
| `--radius-xs` | `0.45rem`   | small chips, code pills |
| `--radius-sm` | `0.9rem`   | inputs, flow-step inner |
| `--radius-md` | `1.15rem`   | nav items, small cards |
| `--radius-lg` | `1.4rem`    | header, default cards |
| `--radius-xl` | `1.6rem`    | large feature cards, consent band |
| `--radius-pill` | `999px`   | pills, chips, dots, coral mark |

---

## ICONOGRAPHY

The ChakraMCP brand is **deliberately icon-light**. The codebase uses zero icon libraries — visual hierarchy comes from typography, color, and simple shapes (a coral circle next to the brand mark; numbered step rings; rotated note-badges).

**When icons are needed**, use **[Phosphor Icons](https://phosphoricons.com/)**, **Regular** weight, 24px grid, 1.5px stroke, outlined (never filled), rounded caps and joins. Phosphor is wider and rounder than Lucide or Heroicons — which gives it editorial personality rather than generic-dashboard energy.

**Inline SVG usage** — each kit embeds the curated Phosphor Regular paths as JSON in a `<script id="__phosphor_icons" type="application/json">` tag; the `<Icon>` helper renders them as inline `<svg>`. No font cascade, no external icon font.

```jsx
<Icon name="paper-plane-tilt" size={24} />
```
Or load individual SVGs from `https://unpkg.com/@phosphor-icons/core/assets/regular/<name>.svg`.

**Substitution flag**: Phosphor is linked from CDN; no icon files are bundled in this design system. If offline usage is needed, copy the `@phosphor-icons/core/assets/regular` folder into `assets/icons/`.

**Emoji**: never. Not in UI, not in marketing, not in tags. The brand leans on typographic emphasis + color chips.

**Unicode as icons**: sparingly. Em-dash (`—`), middot (`·`), arrow (`→`) appear in copy but are not treated as icons.

**SVG brand marks**: the original `favicon.svg` in `app/public` uses a purple chevron glyph — this is **NOT** the ChakraMCP brand mark. A new coral-dot + wordmark lockup lives in `assets/logo/` (see `Brand` cards in the Design System tab).

---

## CAVEATS

- **Fonts**: Archivo Expanded + Afacad are loaded from Google Fonts, not bundled. Confirmed OK — no licensed TTFs to ship.
- **Icons**: Phosphor Regular assets embedded as inline SVG in each kit (`ui_kits/app/icons.json`, `ui_kits/website/icons.json`). Selected and ready to ship — no CDN, no font cascade.
- **Logo**: six directions live in `preview/Logo options.html`. Awaiting pick to refine into a final lockup.
- **App UI kit** is **Discover · Chat · Connect** — a frontend for humans to discover agents, test them, and plug their own MCP endpoint in. Real agents never touch this UI; they hit the relay over MCP directly.
- **No bitmap imagery** exists in the source codebase. Brand is typography + color + printed flourishes.
- **No deck**: the portfolio site doubles as the deck.
- **Codebase uses "Agent Telepathy" name**; this system migrates everything to **ChakraMCP**.

See `SKILL.md` for agent-skill metadata. Jump into the **Design System** tab to flip through every token card, component, and screen.
