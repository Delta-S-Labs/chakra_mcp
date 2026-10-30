---
name: chakramcp-design
description: Use this skill to generate well-branded interfaces and assets for ChakraMCP, either for production or throwaway prototypes/mocks/etc. Contains essential design guidelines, colors, type, fonts, assets, and UI kit components for prototyping.
user-invocable: true
---

Read the README.md file within this skill, and explore the other available files.
If creating visual artifacts (slides, mocks, throwaway prototypes, etc), copy assets out and create static HTML files for the user to view. If working on production code, you can copy assets and read the rules here to become an expert in designing with this brand.
If the user invokes this skill without any other guidance, ask them what they want to build or design, ask some questions, and act as an expert designer who outputs HTML artifacts _or_ production code, depending on the need.

# Quick orientation

- **Brand**: ChakraMCP — a relay network for AI agents. Editorial, warm, rogue-zine energy. Not a dashboard, not dark mode.
- **Colors**: a flat stone ground (`oklch(93.6% 0.005 95)`), ink (`oklch(24% 0.03 18)`) and coral (`oklch(63% 0.19 28)`), with lime and butter for small highlights. Whole sections in ink or coral break up the page. No yellow cream (cream with coral reads as an AI-assistant brand). No blue. No gradients, shadows or hairlines. No neon.
- **Type**: Geist for display and text. Headlines are large, regular weight (400) and tightly tracked (about -0.05em); text is Geist 400. Geist Mono for code only. Sentence case everywhere: no small uppercase labels. The wordmark SVG keeps its Archivo Expanded lettering.
- **Icons**: Phosphor Regular via CDN. No emoji, ever.
- **Voice**: short declarative sentences, mischievous, metaphors rooted in physical venues. "The relay is the bouncer." Never "empower" or "leverage."
- **Design tokens**: `colors_and_type.css` — import this once per page and use CSS vars.

# Where to find things

- **Tokens**: `colors_and_type.css` at the skill root
- **Logos + icons**: `assets/`
- **Preview cards** (authoritative visual reference): `preview/` — typography, color, spacing, components, brand
- **Full UI recreations**: `ui_kits/website/` (marketing site), `ui_kits/app/` (product UI)
- **Voice / tone deep dive**: `CONTENT FUNDAMENTALS` section in `README.md`
- **Reference codebase** (read-only context): `app/` — shipped under earlier name "Agent Telepathy"
