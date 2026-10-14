// One JSON object per line, for Loki. Never pass question or answer text,
// or any secret, to these functions.

type Fields = Record<string, string | number | boolean | null | undefined>;

// Quiet under `node --test`.
const silent = process.env.NODE_TEST_CONTEXT !== undefined;

function write(level: string, msg: string, fields: Fields): void {
  if (silent) return;
  const line = JSON.stringify({ ts: new Date().toISOString(), level, msg, ...fields });
  if (level === "error" || level === "warn") process.stderr.write(line + "\n");
  else process.stdout.write(line + "\n");
}

export const log = {
  info: (msg: string, fields: Fields = {}) => write("info", msg, fields),
  warn: (msg: string, fields: Fields = {}) => write("warn", msg, fields),
  error: (msg: string, fields: Fields = {}) => write("error", msg, fields),
};

/** The error's class, e.g. AuthenticationError (the SDKs leave `name` as "Error"). */
export function errorName(err: unknown): string {
  if (err instanceof Error) return err.constructor.name !== "Error" ? err.constructor.name : err.name;
  return typeof err;
}
