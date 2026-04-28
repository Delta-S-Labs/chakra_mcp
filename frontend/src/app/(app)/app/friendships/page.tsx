import { auth } from "@/auth";
import { listMyAgents, listNetworkAgents, listFriendships } from "@/lib/relay";
import { ProposeForm } from "./ProposeForm";
import { FriendshipsList } from "./FriendshipsList";
import styles from "./friendships.module.css";

/**
 * /app/friendships - agent-to-agent social ties.
 *
 * A friendship is the "we know each other" gate. It's a prerequisite
 * for grants (milestone C); the relay won't deliver invocations
 * between agents that haven't accepted each other.
 */
export default async function FriendshipsPage() {
  const session = await auth();
  const token = session?.backendToken;

  let mine: Awaited<ReturnType<typeof listMyAgents>> = [];
  let network: Awaited<ReturnType<typeof listNetworkAgents>> = [];
  let friendships: Awaited<ReturnType<typeof listFriendships>> = [];
  let backendError: string | null = null;

  if (token) {
    try {
      const [m, n, f] = await Promise.all([
        listMyAgents(token),
        listNetworkAgents(token),
        listFriendships(token, { direction: "all" }),
      ]);
      mine = m;
      network = n;
      friendships = f;
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  }

  const inbound = friendships.filter((f) => f.i_received && f.status === "proposed");
  const outbound = friendships.filter((f) => f.i_proposed && f.status === "proposed");
  const accepted = friendships.filter((f) => f.status === "accepted");
  const history = friendships.filter((f) =>
    ["rejected", "cancelled", "countered"].includes(f.status),
  );

  // Candidates Bob can address from each of his agents - anything on the
  // network that isn't himself.
  const candidates = network.filter((a) => !a.is_mine);

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Friendships</div>
        <h1 className={styles.title}>Who do your agents know?</h1>
        <p className={styles.body}>
          A friendship is a yes/no social tie between two agents. Once
          accepted, you can issue grants (next milestone) that say which
          capabilities each side can call. Counter a proposal if you
          want to rewrite the message before accepting.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <ProposeForm token={token ?? null} myAgents={mine} candidates={candidates} />

      <FriendshipsList
        token={token ?? null}
        title="Inbound"
        subtitle="Other agents proposing to yours."
        items={inbound}
        empty="Nothing inbound right now."
      />

      <FriendshipsList
        token={token ?? null}
        title="Outbound"
        subtitle="Proposals you've sent that are still in flight."
        items={outbound}
        empty="No pending outbound proposals."
      />

      <FriendshipsList
        token={token ?? null}
        title="Accepted"
        subtitle="Live ties. Grants can reference these."
        items={accepted}
        empty="No accepted friendships yet."
      />

      {history.length > 0 && (
        <FriendshipsList
          token={token ?? null}
          title="History"
          subtitle="Rejected, cancelled, and countered."
          items={history}
          empty=""
        />
      )}
    </div>
  );
}
