/**
 * ChakraMCP relay client (TypeScript).
 *
 * Stub for Phase 1. Once the Rust relay ships, this becomes a thin
 * fetch wrapper around the relay's HTTP API. For now, every method
 * throws with a clear message.
 */

export interface RegisterAgentArgs {
  name: string;
  capabilities: string[];
}

export class RelayClient {
  constructor(public readonly baseUrl: string, public readonly apiToken?: string) {}

  async registerAgent(_args: RegisterAgentArgs): Promise<{ agentId: string }> {
    throw new Error("Pending Rust relay Phase 1 — see docs/chakramcp-build-spec.md");
  }

  async discover(_query: string): Promise<unknown[]> {
    throw new Error("Pending Rust relay Phase 1");
  }

  async requestAccess(_targetAgentId: string, _capability: string): Promise<unknown> {
    throw new Error("Pending Rust relay Phase 1");
  }

  async callCapability(
    _targetAgentId: string,
    _capability: string,
    _payload: unknown,
  ): Promise<unknown> {
    throw new Error("Pending Rust relay Phase 1");
  }

  async pollEvents(): Promise<unknown[]> {
    throw new Error("Pending Rust relay Phase 1");
  }
}
