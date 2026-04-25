import Link from "next/link";
import { notFound } from "next/navigation";
import { auth } from "@/auth";
import { getAgent, listCapabilities, RelayClientError } from "@/lib/relay";
import { EditAgentForm } from "./EditAgentForm";
import { CapabilitiesPanel } from "./CapabilitiesPanel";
import styles from "../agents.module.css";

export default async function AgentDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const session = await auth();
  const token = session?.backendToken ?? null;
  if (!token) {
    return <div className={styles.error}>Sign in again — no backend token in this session.</div>;
  }

  let agent: Awaited<ReturnType<typeof getAgent>>;
  try {
    agent = await getAgent(token, id);
  } catch (err) {
    if (err instanceof RelayClientError && err.status === 404) {
      notFound();
    }
    throw err;
  }

  let capabilities: Awaited<ReturnType<typeof listCapabilities>> = [];
  try {
    capabilities = await listCapabilities(token, id);
  } catch {
    // non-fatal — show the header anyway
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href="/app/agents" className={styles.backLink}>
            ← Agents
          </Link>
        </div>
        <h1 className={styles.title}>
          {agent.display_name}{" "}
          <span
            className={`${styles.badge} ${
              agent.visibility === "network" ? styles.badgeOn : ""
            }`}
          >
            {agent.visibility}
          </span>
        </h1>
        <p className={styles.body}>
          <code>
            {agent.account_slug}/{agent.slug}
          </code>{" "}
          · owned by <strong>{agent.account_display_name}</strong>
        </p>
        {agent.description && <p className={styles.body}>{agent.description}</p>}
      </header>

      {agent.is_mine ? (
        <EditAgentForm token={token} agent={agent} />
      ) : (
        <div className={styles.notice}>
          You&apos;re viewing this agent as a network visitor. Editing is
          limited to members of <strong>{agent.account_display_name}</strong>.
        </div>
      )}

      <CapabilitiesPanel
        token={token}
        agentId={agent.id}
        canEdit={agent.is_mine}
        initial={capabilities}
      />
    </div>
  );
}
