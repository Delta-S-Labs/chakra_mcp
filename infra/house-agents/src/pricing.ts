// What a Claude call cost, from its usage. Claude Sonnet 5.5, US dollars per
// million tokens. Cache writes are the 5-minute kind.
export const PRICE_PER_MTOK = {
  input: 2,
  output: 10,
  cacheWrite: 2.5,
  cacheRead: 0.2,
} as const;

export interface TokenUsage {
  input_tokens?: number | null;
  output_tokens?: number | null;
  cache_creation_input_tokens?: number | null;
  cache_read_input_tokens?: number | null;
}

export interface UsageWithIterations extends TokenUsage {
  /** Compaction runs as its own iteration; the top-level counts leave it out. */
  iterations?: ReadonlyArray<TokenUsage & { type?: string }> | null;
}

function tokensUsd(u: TokenUsage): number {
  return (
    ((u.input_tokens ?? 0) * PRICE_PER_MTOK.input +
      (u.output_tokens ?? 0) * PRICE_PER_MTOK.output +
      (u.cache_creation_input_tokens ?? 0) * PRICE_PER_MTOK.cacheWrite +
      (u.cache_read_input_tokens ?? 0) * PRICE_PER_MTOK.cacheRead) /
    1_000_000
  );
}

export function costUsd(usage: UsageWithIterations): number {
  if (usage.iterations && usage.iterations.length > 0) {
    return usage.iterations.reduce((sum, it) => sum + tokensUsd(it), 0);
  }
  return tokensUsd(usage);
}

/**
 * The most a call can have cost when it failed without reporting usage
 * (a timeout, a dropped stream): the whole prompt written to the cache,
 * plus a compaction pass over it, plus a full answer.
 */
export function worstCaseUsd(promptChars: number, maxTokens: number): number {
  // About 3 characters a token for English and JSON.
  const promptTokens = Math.ceil(promptChars / 3);
  return (
    (promptTokens * PRICE_PER_MTOK.cacheWrite * 2 + maxTokens * PRICE_PER_MTOK.output * 2) /
    1_000_000
  );
}
