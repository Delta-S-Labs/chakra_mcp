import Link from "next/link";
import { notFound } from "next/navigation";
import { auth } from "@/auth";
import SafetyNote from "@/components/safety/SafetyNote";
import {
  getCall,
  getCallMemory,
  listCallMessages,
  listMyAgents,
  RelayClientError,
  type Call,
  type CallEntry,
  type CallMemory,
  type CallMemoryItem,
} from "@/lib/relay";
import { redirectIfUnauthorized } from "@/lib/session-expired";
import {
  canDelete,
  canEnd,
  endedLabel,
  eventLine,
  memoryLabel,
  partyName,
  retentionLabel,
  senderOf,
  stateLabel,
} from "@/lib/calls";
import { LocalTime } from "../../audit/LocalTime";
import { OwnerActions } from "./OwnerActions";
import styles from "../calls.module.css";

/** The most of a call's transcript shown at once. */
const LIMIT = 200;

/** /app/calls/[id] - one call: its thread, its memory, and owner actions. */
export default async function CallPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const session = await auth();
  const token = session?.backendToken ?? null;
  if (!token) {
    return <div className={styles.error}>Sign in again - no backend token in this session.</div>;
  }

  let call: Call;
  let entries: CallEntry[];
  let mine: Set<string>;
  try {
    [call, entries, mine] = await Promise.all([
      getCall(token, id),
      listCallMessages(token, id, { limit: LIMIT }),
      listMyAgents(token).then((agents) => new Set(agents.map((a) => a.id))),
    ]);
  } catch (err) {
    redirectIfUnauthorized(err, `/app/calls/${id}`);
    if (err instanceof RelayClientError && err.status === 404) notFound();
    throw err;
  }
  let memory: CallMemory | null = null;
  // Deleted content leaves no memory to show.
  if (call.memory === "call" && !call.purged_at) {
    memory = await getCallMemory(token, id).catch(() => null);
  }

  return (
    <div className={styles.page}>
      <Link href="/app/calls" className={styles.back}>
        ← All calls
      </Link>
      <header className={styles.head}>
        <div className="eyebrow">Call · {stateLabel(call.state)}</div>
        <h1 className={styles.title}>
          {partyName(call.caller)} and {partyName(call.callee)}
        </h1>
        {call.topic && <p className={styles.body}>{call.topic}</p>}
      </header>

      <section className={styles.terms} aria-label="Terms">
        <span>
          Started <LocalTime iso={call.created_at} /> · {call.message_count} messages
        </span>
        <span>{memoryLabel(call)}</span>
        {endedLabel(call) && <span>{endedLabel(call)}</span>}
        <span>{retentionLabel(call)}</span>
        <OwnerActions callId={call.id} canEnd={canEnd(call)} canDelete={canDelete(call)} />
      </section>

      <div className={styles.layout}>
        <ol className={styles.thread} aria-label="Transcript">
          {entries.map((e) => (
            <Entry key={e.seq} call={call} entry={e} mine={mine} />
          ))}
          {entries.length === LIMIT && (
            <li className={styles.event}>Showing the first {LIMIT} entries.</li>
          )}
        </ol>
        {memory && <MemoryPanel call={call} memory={memory} />}
      </div>
    </div>
  );
}

function Entry({ call, entry, mine }: { call: Call; entry: CallEntry; mine: Set<string> }) {
  if (entry.kind !== "message") {
    return <li className={styles.event}>{eventLine(call, entry)}</li>;
  }
  const sender = senderOf(call, entry);
  const isMine = !!entry.from_agent_id && mine.has(entry.from_agent_id);
  return (
    <li className={`${styles.msg} ${isMine ? styles.msgMine : ""}`}>
      <span className={styles.msgFrom}>{sender ? partyName(sender) : "An agent"}</span>
      {entry.blocked ? (
        <p className={styles.msgGone}>Blocked by safety screening. The content isn&apos;t kept.</p>
      ) : entry.text === null ? (
        <p className={styles.msgGone}>Content deleted.</p>
      ) : (
        <p className={styles.msgText}>{entry.text}</p>
      )}
      <SafetyNote findings={entry.safety} />
      <span className={styles.msgTime}>
        <LocalTime iso={entry.created_at} />
      </span>
    </li>
  );
}

function MemoryPanel({ call, memory }: { call: Call; memory: CallMemory }) {
  const item = (m: CallMemoryItem) => {
    const sender =
      m.from === call.caller.agent_id
        ? call.caller
        : m.from === call.callee.agent_id
          ? call.callee
          : null;
    return (
      <li key={m.seq}>
        <strong>{sender ? partyName(sender) : "An agent"}:</strong> {m.text}
        {m.context && <span className={styles.context}>In reply to: {m.context}</span>}
      </li>
    );
  };
  return (
    <aside className={styles.panel} aria-label="Call memory">
      <h2 className={styles.panelTitle}>Memory</h2>
      <p className={styles.body}>
        What the relay keeps for the agents, verbatim: decisions, commitments and facts
        from earlier, and questions not yet answered.
      </p>
      {!call.memory_features.tags && (
        <p className={styles.empty}>Only the latest messages: memory was off when it started.</p>
      )}
      {memory.pinned.length > 0 && (
        <>
          <h3 className={styles.panelTitle}>Pinned</h3>
          <ul className={styles.panelList}>{memory.pinned.map(item)}</ul>
        </>
      )}
      {memory.open.length > 0 && (
        <>
          <h3 className={styles.panelTitle}>Open questions</h3>
          <ul className={styles.panelList}>{memory.open.map(item)}</ul>
        </>
      )}
      {memory.pinned.length === 0 && memory.open.length === 0 && call.memory_features.tags && (
        <p className={styles.empty}>Nothing pinned yet: the latest messages are in the thread.</p>
      )}
    </aside>
  );
}
