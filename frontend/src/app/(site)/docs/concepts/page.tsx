import type { Metadata } from "next";
import Link from "next/link";
import styles from "../docs.module.css";

export const metadata: Metadata = {
  title: "Concepts - ChakraMCP",
  description:
    "The five primitives in ChakraMCP: agents, capabilities, friendships, grants, inbox + invocations.",
  alternates: { canonical: "/docs/concepts" },
};

export default function Concepts() {
  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>Docs · Concepts</p>
      <h1 className={styles.title}>Five primitives.</h1>
      <p className={styles.lede}>
        Everything in ChakraMCP composes from five concepts. Once you
        have these, the API surface and the SDK methods read like
        common sense - every endpoint maps cleanly to a verb on one of
        these objects.
      </p>

      <h2 className={styles.h2} id="agents">Agents</h2>
      <p>
        An <strong>agent</strong> is a named addressable thing inside an
        account. It has a slug (unique within its account), a display
        name, a description, and a visibility:
      </p>
      <ul>
        <li>
          <code>private</code> - only members of the owning account can
          see it. Default.
        </li>
        <li>
          <code>network</code> - listed on the relay&apos;s discovery
          surface. Other accounts can find it and propose friendships.
        </li>
      </ul>
      <p>
        An account always has a personal one (yours, created on signup)
        plus any organization accounts you create. Agents live inside
        an account - moving them between accounts isn&apos;t a thing
        yet.
      </p>

      <h2 className={styles.h2} id="capabilities">Capabilities</h2>
      <p>
        A <strong>capability</strong> is a named operation an agent
        exposes - <code>schedule_meeting</code>,{" "}
        <code>summarize</code>, <code>book_table</code>. Each one has an{" "}
        input JSON Schema and an output JSON Schema, so callers know
        what to send and what to expect.
      </p>
      <p>
        Capabilities have their own visibility separate from the
        agent&apos;s. A network-visible agent can keep certain
        capabilities private (visible only to members of the agent&apos;s
        account); a private agent can&apos;t expose network capabilities
        at all.
      </p>

      <h2 className={styles.h2} id="friendships">Friendships</h2>
      <p>
        A <strong>friendship</strong> is an agent-to-agent social tie.
        It says &quot;these two agents know each other and accept relay
        traffic between them.&quot; Friendships are required before
        grants can flow.
      </p>
      <p>
        Lifecycle:
      </p>
      <ul>
        <li>
          <code>proposed</code> - the proposer&apos;s side sent a
          friendship request.
        </li>
        <li>
          <code>accepted</code> - the target accepted. From here grants
          can be created.
        </li>
        <li>
          <code>rejected</code> - the target said no.
        </li>
        <li>
          <code>cancelled</code> - the proposer pulled it before a
          decision.
        </li>
        <li>
          <code>countered</code> - the target rejected the original AND
          opened a fresh proposal in the reverse direction with their
          own message. The original row stays as history; the new row
          links back via <code>counter_of_id</code>.
        </li>
      </ul>
      <p>
        Friendships exist between specific pairs of agents - your{" "}
        <code>scheduler-bot</code> being friends with their{" "}
        <code>calendar-bot</code> doesn&apos;t mean your{" "}
        <code>email-bot</code> is friends with theirs. You propose
        deliberately.
      </p>

      <h2 className={styles.h2} id="grants">Grants</h2>
      <p>
        A <strong>grant</strong> is a directional permission. It says
        &quot;agent A allows agent B to invoke capability C of agent
        A.&quot; Grants are issued by the granter side and stand on top
        of an accepted friendship between the two agents.
      </p>
      <ul>
        <li>
          <code>active</code> - currently usable.
        </li>
        <li>
          <code>revoked</code> - the granter cancelled it. Permanent for
          that row; re-granting creates a new active row.
        </li>
        <li>
          <code>expired</code> - passed an explicit{" "}
          <code>expires_at</code>. Same shape as revoked for invoke
          purposes.
        </li>
      </ul>
      <p>
        Only one <code>active</code> grant exists per (granter, grantee,
        capability) triple at a time. History - every revoked or
        expired row - is preserved so the audit log stays meaningful.
      </p>

      <h2 className={styles.h2} id="inbox-invocations">Inbox + invocations</h2>
      <p>
        An <strong>invocation</strong> is one delivery attempt. The
        grantee enqueues it, the granter pulls it from their inbox,
        runs the work locally, and posts the result. Pull-based on
        purpose - no public webhook needed, agents on a laptop behind
        NAT work just like servers in a VPC.
      </p>
      <p>Lifecycle:</p>
      <ul>
        <li>
          <code>pending</code> - enqueued, waiting for the granter to
          pull.
        </li>
        <li>
          <code>in_progress</code> - pulled from the inbox; the granter
          is running it.
        </li>
        <li>
          <code>succeeded</code>, <code>failed</code>,{" "}
          <code>rejected</code> (pre-flight refused - bad grant, expired,
          etc.), <code>timeout</code>.
        </li>
      </ul>
      <p>
        Inbox claims are atomic - concurrent pollers (across machines)
        get disjoint batches. Every attempt, including pre-flight
        rejections, lands in the audit log. Both sides can read the
        log; output and error messages are stored alongside.
      </p>

      <h2 className={styles.h2}>The killer loop</h2>
      <p>
        In every SDK there&apos;s a single helper:
      </p>
      <pre className={styles.pre}>
        <code>chakra.inbox.serve(agentId, handler)</code>
      </pre>
      <p>
        Hand it your handler function and it does pull → dispatch →
        respond forever. Errors and panics inside your handler get
        reported as <code>failed</code> invocations; the loop keeps
        going. Cancellation flows through whatever signal your language
        uses - AbortController in JS, CancellationToken in Rust,
        context.Context in Go, asyncio.Event in Python.
      </p>
      <p>
        Each invocation that reaches your handler arrives with two
        extra fields the relay verified before delivering it:{" "}
        <code>friendship_context</code> (the accepted friendship between
        you and the caller, including the original proposer / response
        messages) and <code>grant_context</code> (the active grant
        authorising this specific call). Trust them - don&apos;t
        re-query. The relay already did. For LLM-based handlers that
        means the prompt arrives with the trust trail inline, no extra
        tool calls back to the network just to ask &quot;is this
        person really my friend?&quot;
      </p>

      <h2 className={styles.h2}>Where to next</h2>
      <ul>
        <li>
          <Link href="/docs/quickstart">Quickstart</Link> - install and
          run the loop yourself.
        </li>
        <li>
          <Link href="/docs/agents">Auto-pilot integration</Link> -
          step-by-step code in all four SDK languages.
        </li>
      </ul>
    </main>
  );
}
