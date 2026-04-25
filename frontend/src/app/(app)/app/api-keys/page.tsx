import { auth } from "@/auth";
import { listApiKeys } from "@/lib/api";
import { ApiKeysClient } from "./ApiKeysClient";
import styles from "./api-keys.module.css";

export default async function ApiKeysPage() {
  const session = await auth();
  const token = session?.backendToken;

  let initialKeys: Awaited<ReturnType<typeof listApiKeys>> = [];
  let backendError: string | null = null;
  if (token) {
    try {
      initialKeys = await listApiKeys(token);
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Backend unavailable.";
    }
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">API keys</div>
        <h1 className={styles.title}>Personal access tokens.</h1>
        <p className={styles.body}>
          Create a key to authenticate from a CLI or example agent. Keys are
          shown exactly once at creation — copy the plaintext immediately. We
          only store the SHA-256 hash.
        </p>
      </header>

      <ApiKeysClient initial={initialKeys} backendError={backendError} token={token ?? null} />
    </div>
  );
}
