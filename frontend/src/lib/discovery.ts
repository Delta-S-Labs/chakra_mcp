// The public directory's data: the relay's discovery responses (MCP and
// discovery spec §5), fetched for the /agents pages, and small helpers for
// showing them.

const RELAY_BASE = process.env.NEXT_PUBLIC_RELAY_URL ?? "http://localhost:8090";

/** Who can call a capability: anyone signed in (`public`), or friends with a grant. */
export type Access = "public" | "friends";

export interface CapabilityBrief {
  id: string;
  name: string;
  access: Access;
}

/** The fields every directory list shares. */
export interface AgentListing {
  id: string;
  slug: string;
  account_slug: string;
  display_name: string;
  description: string;
  mode: "push" | "pull";
  tags: string[];
  visibility: "private" | "org" | "network";
  online: boolean;
  last_seen_at: string;
  created_at: string;
  verified: boolean;
  featured: boolean;
  friend_count: number;
  avg_rating: number | null;
  review_count: number;
  accept_calls: "friends" | "anyone" | "nobody";
}

/** A signed-in viewer's standing with an agent. */
export interface Standing {
  callable: {
    capability: string;
    capability_id: string;
    via: "grant" | "public";
    as_agent_id: string | null;
  }[];
  friendships: { id: string; your_agent_id: string; status: "proposed" | "accepted" }[];
  can_start_call_as: string[];
}

export interface DirectoryAgent extends AgentListing {
  capabilities: CapabilityBrief[];
}

export interface DirectoryPage {
  agents: DirectoryAgent[];
  next_cursor: string | null;
  total_estimate?: number;
}

export interface CapabilityDetail {
  id: string;
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  output_schema: Record<string, unknown>;
  access: Access;
  public_monthly_quota_per_agent: number | null;
  consent_required: boolean;
  semantics: "autonomous" | "human_in_loop";
}

export interface AgentProfile extends AgentListing {
  owner: { account_slug: string; display_name: string; verified: boolean };
  reviews: {
    avg_rating: number | null;
    count: number;
    latest: { rating: number; text: string | null; at: string; by: string }[];
  };
  capabilities: CapabilityDetail[];
  how_to_call: { cli: string; a2a_card?: string; profile?: string; mcp?: string };
  you?: Standing;
}

/**
 * An agent's profile, or `null` when there's no such agent the viewer may
 * see (or the relay can't be reached). Signed out it's cached for 30
 * seconds, as the directory is; with a token it's fetched fresh and
 * carries the viewer's standing. A token the relay refuses falls back to
 * the signed-out view.
 */
export async function fetchProfile(
  accountSlug: string,
  agentSlug: string,
  token?: string | null,
): Promise<AgentProfile | null> {
  const url = `${RELAY_BASE}/v1/discovery/agents/${encodeURIComponent(
    accountSlug,
  )}/${encodeURIComponent(agentSlug)}`;
  try {
    if (token) {
      const res = await fetch(url, {
        headers: { authorization: `Bearer ${token}` },
        cache: "no-store",
      });
      if (res.ok) return (await res.json()) as AgentProfile;
      if (res.status !== 401) return null;
    }
    const res = await fetch(url, { next: { revalidate: 30 } });
    return res.ok ? ((await res.json()) as AgentProfile) : null;
  } catch (err) {
    console.error(`[agent profile] fetch failed: ${url}`, err);
    return null;
  }
}

/** How long ago `iso` was, coarsely: "just now", "5m ago", "3h ago", "2d ago", "4mo ago". */
export function formatAgo(iso: string, now: number = Date.now()): string {
  const ms = now - Date.parse(iso);
  if (!Number.isFinite(ms) || ms < 60_000) return "just now";
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 60) return `${days}d ago`;
  return `${Math.floor(days / 30)}mo ago`;
}

/** Tags the agent edit form suggests (spec §5.6); any tag in the format is allowed. */
export const TAG_SUGGESTIONS = [
  "scheduling",
  "calendar",
  "travel",
  "booking",
  "coding",
  "devops",
  "research",
  "writing",
  "translation",
  "finance",
  "sales",
  "support",
  "marketing",
  "data",
  "analytics",
  "health",
  "education",
  "legal",
  "shopping",
  "personal-assistant",
] as const;

export const MAX_TAGS = 10;

/**
 * Tags typed as text ("travel, Booking  b2b"): split on commas and spaces,
 * lowercased, duplicates dropped. `problem` says what the relay would
 * refuse, so the form can say it first.
 */
export function parseTags(text: string): { tags: string[]; problem?: string } {
  const tags: string[] = [];
  for (const raw of text.split(/[\s,]+/)) {
    const tag = raw.trim().toLowerCase();
    if (tag && !tags.includes(tag)) tags.push(tag);
  }
  const bad = tags.find((t) => !/^[a-z0-9-]{1,32}$/.test(t));
  if (bad) {
    return { tags, problem: `"${bad}": tags are 1-32 lowercase letters, digits or hyphens.` };
  }
  if (tags.length > MAX_TAGS) {
    return { tags, problem: `At most ${MAX_TAGS} tags.` };
  }
  return { tags };
}
