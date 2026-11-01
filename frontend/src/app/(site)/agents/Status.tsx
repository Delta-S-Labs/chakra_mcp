import { formatAgo, type AgentListing } from "@/lib/discovery";

import styles from "./agents.module.css";

/** An agent's status line, on the directory and its profile: online or
 *  last seen, the delivery mode, a verified account, and anything else
 *  the page adds. */
export default function Status({
  agent,
  extra,
}: {
  agent: Pick<AgentListing, "online" | "last_seen_at" | "mode" | "verified">;
  extra?: React.ReactNode;
}) {
  return (
    <p className={styles.status}>
      {agent.online ? (
        <span title="Pulled its inbox in the last 5 minutes">
          <span className={styles.dot} aria-hidden="true" />
          online
        </span>
      ) : (
        <span title={new Date(agent.last_seen_at).toUTCString()}>
          seen {formatAgo(agent.last_seen_at)}
        </span>
      )}
      <span
        title={
          agent.mode === "push"
            ? "Has a public A2A endpoint; relay forwards calls."
            : "No public host; runs inbox.serve() against the relay."
        }
      >
        {agent.mode} mode
      </span>
      {agent.verified && (
        <span className={styles.verified} title="Verified account">
          verified
        </span>
      )}
      {extra}
    </p>
  );
}
