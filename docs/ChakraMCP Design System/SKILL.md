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
- **Colors**: cream paper (`oklch(96% 0.014 72)`), ink (`oklch(24% 0.03 18)`), + coral / lime / butter accents. No blue. No purple-to-blue gradients. No neon.
- **Type**: Archivo Expanded (display, heavy, wide, negative tracking) + Afacad (body) + system mono. Avoid Inter, Geist, DM Sans, Roboto.
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
