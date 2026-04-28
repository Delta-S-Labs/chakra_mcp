"use client";

import { useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  createGrant,
  listCapabilities,
  type Agent,
  type Capability,
  type Friendship,
} from "@/lib/relay";
import styles from "./grants.module.css";

export function CreateGrantForm({
  token,
  myAgents,
  acceptedFriendships,
}: {
  token: string | null;
  myAgents: Agent[];
  acceptedFriendships: Friendship[];
}) {
  const router = useRouter();

  // Each of my agents has a set of network friends - agents on the other
  // side of an accepted friendship. Keyed by my-agent id.
  const friendsByAgent = useMemo(() => {
    const m = new Map<string, Array<{ id: string; label: string }>>();
    for (const a of myAgents) m.set(a.id, []);
    for (const f of acceptedFriendships) {
      // figure out which side is "mine" for this friendship
      const myProposer = myAgents.find((a) => a.id === f.proposer.id);
      const myTarget = myAgents.find((a) => a.id === f.target.id);
      if (myProposer) {
        m.get(myProposer.id)!.push({
          id: f.target.id,
          label: `${f.target.display_name} · ${f.target.account_display_name}`,
        });
      }
      if (myTarget) {
        m.get(myTarget.id)!.push({
          id: f.proposer.id,
          label: `${f.proposer.display_name} · ${f.proposer.account_display_name}`,
        });
      }
    }
    return m;
  }, [myAgents, acceptedFriendships]);

  const granterCandidates = useMemo(
    () => myAgents.filter((a) => (friendsByAgent.get(a.id)?.length ?? 0) > 0),
    [myAgents, friendsByAgent],
  );

  const [granterId, setGranterId] = useState(granterCandidates[0]?.id ?? "");
  const [granteeId, setGranteeId] = useState("");
  const [capId, setCapId] = useState("");
  const [capabilities, setCapabilities] = useState<Capability[]>([]);
  const [loadingCaps, setLoadingCaps] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // When granter changes, refresh its capability list and seed grantee.
  useEffect(() => {
    if (!token || !granterId) return;
    let cancelled = false;
    setLoadingCaps(true);
    setError(null);
    listCapabilities(token, granterId)
      .then((rows) => {
        if (cancelled) return;
        setCapabilities(rows);
        setCapId(rows[0]?.id ?? "");
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err.message : "Couldn't load capabilities.");
      })
      .finally(() => !cancelled && setLoadingCaps(false));
    const friends = friendsByAgent.get(granterId) ?? [];
    setGranteeId(friends[0]?.id ?? "");
    return () => {
      cancelled = true;
    };
  }, [token, granterId, friendsByAgent]);

  if (myAgents.length === 0) {
    return (
      <div className={styles.notice}>
        Register an agent first under <strong>Agents</strong> before issuing
        grants.
      </div>
    );
  }
  if (granterCandidates.length === 0) {
    return (
      <div className={styles.notice}>
        Your agents don&apos;t have any accepted friendships yet. Head to{" "}
        <strong>Friendships</strong> and propose one - friendships are the
        prerequisite for granting capability access.
      </div>
    );
  }

  const granteeOptions = friendsByAgent.get(granterId) ?? [];

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!token) {
      setError("Sign in again - no backend token.");
      return;
    }
    if (!granteeId || !capId) {
      setError("Pick a grantee and a capability.");
      return;
    }
    setError(null);
    setPending(true);
    try {
      await createGrant(token, {
        granter_agent_id: granterId,
        grantee_agent_id: granteeId,
        capability_id: capId,
      });
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't create grant.");
    } finally {
      setPending(false);
    }
  }

  return (
    <section className={styles.createPanel}>
      <header className={styles.formHead}>
        <h2 className={styles.sectionTitle}>Issue a grant</h2>
        <p className={styles.formHint}>
          Pick one of your agents, the friend it&apos;s granting to, and a
          capability of yours to expose. The grantee can then invoke that
          capability through the relay.
        </p>
      </header>

      <form className={styles.fields} onSubmit={handleSubmit}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>From your agent</span>
          <select value={granterId} onChange={(e) => setGranterId(e.target.value)}>
            {granterCandidates.map((a) => (
              <option key={a.id} value={a.id}>
                {a.display_name}
              </option>
            ))}
          </select>
        </label>

        <label className={styles.field}>
          <span className={styles.fieldLabel}>To friend</span>
          <select
            value={granteeId}
            onChange={(e) => setGranteeId(e.target.value)}
            disabled={granteeOptions.length === 0}
          >
            {granteeOptions.length === 0 ? (
              <option value="">No friends yet</option>
            ) : (
              granteeOptions.map((g) => (
                <option key={g.id} value={g.id}>
                  {g.label}
                </option>
              ))
            )}
          </select>
        </label>

        <label className={styles.field}>
          <span className={styles.fieldLabel}>Capability</span>
          <select
            value={capId}
            onChange={(e) => setCapId(e.target.value)}
            disabled={loadingCaps || capabilities.length === 0}
          >
            {capabilities.length === 0 ? (
              <option value="">{loadingCaps ? "Loading…" : "No capabilities"}</option>
            ) : (
              capabilities.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name} ({c.visibility})
                </option>
              ))
            )}
          </select>
        </label>

        <button type="submit" className={styles.create} disabled={pending}>
          {pending ? "Granting…" : "Grant"}
        </button>
      </form>

      {error && <div className={styles.errorInline}>{error}</div>}
    </section>
  );
}
