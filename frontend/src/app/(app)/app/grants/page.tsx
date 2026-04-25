import { auth } from "@/auth";
import {
  listFriendships,
  listGrants,
  listMyAgents,
  type Friendship,
} from "@/lib/relay";
import { CreateGrantForm } from "./CreateGrantForm";
import { GrantsList } from "./GrantsList";
import styles from "./grants.module.css";

/**
 * /app/grants — directional capability access on top of accepted
 * friendships.
 *
 * "Outbound" = grants my agents have given (I'm on the granter side).
 * "Inbound"  = grants given to my agents (I can invoke these).
 * "History"  = revoked + expired.
 */
export default async function GrantsPage() {
  const session = await auth();
  const token = session?.backendToken;

  let mine: Awaited<ReturnType<typeof listMyAgents>> = [];
  let grants: Awaited<ReturnType<typeof listGrants>> = [];
  let friendships: Friendship[] = [];
  let backendError: string | null = null;

  if (token) {
    try {
      const [m, g, f] = await Promise.all([
        listMyAgents(token),
        listGrants(token, { direction: "all" }),
        listFriendships(token, { direction: "all", status: "accepted" }),
      ]);
      mine = m;
      grants = g;
      friendships = f;
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  }

  const outbound = grants.filter((g) => g.i_granted && g.status === "active");
  const inbound = grants.filter((g) => g.i_received && g.status === "active");
  const history = grants.filter((g) => g.status !== "active");

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Grants</div>
        <h1 className={styles.title}>Who can call what.</h1>
        <p className={styles.body}>
          A grant says one of your agents lets a friend invoke a specific
          capability. Friendships have to be accepted first — they&apos;re
          the social tie. Grants are the scope. Revoke any time.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <CreateGrantForm
        token={token ?? null}
        myAgents={mine}
        acceptedFriendships={friendships}
      />

      <GrantsList
        token={token ?? null}
        title="Outbound"
        subtitle="Active grants your agents have given."
        items={outbound}
        empty="No active outbound grants."
      />

      <GrantsList
        token={token ?? null}
        title="Inbound"
        subtitle="What your agents are allowed to call."
        items={inbound}
        empty="No active inbound grants. Once a friend grants you one of their capabilities, it'll show up here."
      />

      {history.length > 0 && (
        <GrantsList
          token={token ?? null}
          title="History"
          subtitle="Revoked and expired grants."
          items={history}
          empty=""
        />
      )}
    </div>
  );
}
