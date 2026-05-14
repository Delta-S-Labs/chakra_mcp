import Link from "next/link";
import { notFound } from "next/navigation";
import { auth } from "@/auth";
import { getApiKey, getApiKeyUsage } from "@/lib/api";
import { KeyDetailClient } from "./KeyDetailClient";
import styles from "./detail.module.css";

/**
 * /app/api-keys/[id] — single API key, with rotate/revoke + usage
 * charts.
 *
 * The list endpoint is the source of truth for the key row itself
 * (`getApiKey` filters the list). Usage is pulled from
 * `/v1/api-keys/{id}/usage` over the default 30-day window. The
 * endpoint returns zeros today — once the relay invoke handler stamps
 * `api_key_id` on every invocation, real data flows through with no
 * frontend changes.
 */
export default async function ApiKeyDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const session = await auth();
  const token = session?.backendToken;

  if (!token) {
    notFound();
  }

  let key: Awaited<ReturnType<typeof getApiKey>> = null;
  let usage: Awaited<ReturnType<typeof getApiKeyUsage>> | null = null;
  let backendError: string | null = null;

  try {
    [key, usage] = await Promise.all([
      getApiKey(token, id),
      getApiKeyUsage(token, id).catch((err) => {
        // Treat usage failures as non-fatal — the row + actions
        // should still render.
        console.error("[api-keys] usage fetch failed", err);
        return null;
      }),
    ]);
  } catch (err) {
    backendError = err instanceof Error ? err.message : "Backend unavailable.";
  }

  if (!key && !backendError) {
    notFound();
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href="/app/api-keys" className={styles.crumb}>
            ← API keys
          </Link>
        </div>
        <h1 className={styles.title}>{key?.name ?? "API key"}</h1>
        <p className={styles.body}>
          {key ? (
            <>
              <code>{key.prefix}…</code> · created{" "}
              {new Date(key.created_at).toLocaleString()}
              {key.expires_at
                ? ` · expires ${new Date(key.expires_at).toLocaleDateString()}`
                : " · never expires"}
              {key.revoked_at && (
                <>
                  {" · "}
                  <strong className={styles.revokedFlag}>
                    revoked {new Date(key.revoked_at).toLocaleString()}
                  </strong>
                </>
              )}
            </>
          ) : (
            "Key details unavailable."
          )}
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      {key && <KeyDetailClient apiKey={key} initialUsage={usage} token={token} />}
    </div>
  );
}
