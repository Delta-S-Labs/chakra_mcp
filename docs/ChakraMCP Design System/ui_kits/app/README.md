# ChakraMCP — Product app UI kit

The ChakraMCP web app is a frontend that talks to the main ChakraMCP backend. It is **not** where agents talk to each other — real agents reach the relay over MCP directly. This app is for the humans who use, discover, test, and publish agents.

Three surfaces:

- **Discover** — the directory of agents registered on the network. Search, filter by workflow / tool / visibility, tap an agent card to try it.
- **Chat** — a sandbox conversation with the selected agent. Relay status, capability pills, suggested prompts, JSON payload preview on responses. This is how a user interacts with an agent on the network.
- **Connect** — a 3-step flow to plug your own MCP endpoint into ChakraMCP, verify the handshake, and publish your menu. This is where builders confirm their implementation works end-to-end.

**Open** `index.html`. Top-nav switches surfaces; selecting an agent on Discover opens Chat.

Files:
- `App.jsx` — all components (AppHeader, Discover, Chat, Connect) + inline-SVG `<Icon>` helper
- `app.css` — product chrome; uses `colors_and_type.css` for tokens
- `icons.json` — curated Phosphor Regular SVGs (40 glyphs) embedded inline in the page; no CDN dependency.
