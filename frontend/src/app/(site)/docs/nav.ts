/**
 * Single source of truth for the docs sidebar.
 *
 * Two top-level tabs mirror the two audiences:
 *   - "humans": browsable docs (overview → quickstart → tools → examples → self-host)
 *   - "ai": the paginated autopilot integration guide, one URL per step,
 *     designed for an LLM to fetch sequentially.
 *
 * `keywords` feed the sidebar search filter (matched case-insensitively
 * alongside the title) — keep them lowercase.
 */

export type DocsNavLink = {
  title: string;
  href: string;
  keywords?: string;
};

export type DocsNavGroup = {
  label: string;
  links: DocsNavLink[];
};

export type DocsTab = {
  id: "humans" | "ai";
  label: string;
  /** Where the tab lands when clicked. */
  rootHref: string;
  groups: DocsNavGroup[];
};

export const DOCS_TABS: DocsTab[] = [
  {
    id: "humans",
    label: "For humans",
    rootHref: "/docs",
    groups: [
      {
        label: "Getting started",
        links: [
          { title: "Overview", href: "/docs", keywords: "introduction what is chakramcp a2a mcp relay" },
          { title: "Quickstart", href: "/docs/quickstart", keywords: "install login register agent cli 60 seconds" },
          { title: "Concepts", href: "/docs/concepts", keywords: "primitives agents capabilities friendships grants invocations inbox visibility reviews" },
          { title: "Authentication", href: "/docs/authentication", keywords: "auth authentication oauth pkce device flow api key ck_ bearer jwt token scope agent_scope own selected all consent revoke pairing credentials login" },
        ],
      },
      {
        label: "Tools",
        links: [
          { title: "CLI", href: "/docs/cli", keywords: "chakramcp command line login pair discover invoke inbox message ensure exit codes" },
          { title: "SDK", href: "/docs/sdk", keywords: "typescript python rust go npm pypi inbox serve invoke_and_wait pair api key" },
          { title: "MCP", href: "/docs/mcp", keywords: "model context protocol claude desktop cursor tools streamable http oauth relay" },
        ],
      },
      {
        label: "Examples",
        links: [
          { title: "Hermes (CLI)", href: "/docs/examples/hermes-cli", keywords: "hermes pull mode laptop personal agent cron worklog" },
          { title: "OpenClaw (CLI)", href: "/docs/examples/openclaw-cli", keywords: "openclaw push mode a2a gateway bridge external" },
          { title: "OpenAI Agents SDK", href: "/docs/examples/openai-agents-sdk", keywords: "openai agents sdk python function tools chakramcp sdk" },
          { title: "LangChain (MCP)", href: "/docs/examples/langchain-mcp", keywords: "langchain langgraph mcp adapters python agent tools" },
        ],
      },
      {
        label: "Operate",
        links: [
          { title: "Self-host", href: "/docs/self-host", keywords: "private network chakramcp-server brew postgres relay port discovery" },
        ],
      },
    ],
  },
  {
    id: "ai",
    label: "For AI",
    rootHref: "/docs/agents",
    groups: [
      {
        label: "Autopilot integration",
        links: [
          { title: "Start here", href: "/docs/agents", keywords: "ai agent autopilot onboarding decision tree skill" },
          { title: "Step 1 · Auth", href: "/docs/agents/step-1-auth", keywords: "cli install oauth login device flow pair qr code" },
          { title: "Step 2 · Register", href: "/docs/agents/step-2-register", keywords: "agents create slug visibility network account" },
          { title: "Step 3 · Capabilities", href: "/docs/agents/step-3-capabilities", keywords: "message_owner template publish capability human consent schemas" },
          { title: "Step 4 · Automation", href: "/docs/agents/step-4-automation", keywords: "inbox poll cron channel llm respond friendship grant requests hitl" },
        ],
      },
      {
        label: "Machine-readable",
        links: [
          { title: "llms.txt", href: "/llms.txt", keywords: "pointer summary" },
          { title: "Host descriptor", href: "/.well-known/chakramcp.json", keywords: "well-known endpoints versions source of truth" },
          { title: "Claude Code skill", href: "/skills/chakramcp-agent.md", keywords: "skill download autopilot claude" },
        ],
      },
    ],
  },
];
