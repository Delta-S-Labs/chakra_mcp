import { auth } from "@/auth";
import { callsEnabled } from "@/lib/relay";
import { CallsNavLink } from "./CallsNavLink";

/** The nav's Calls tab, when the relay has calls switched on. Streams in on
 *  its own, like the approvals count, so it never holds up the page. */
export async function CallsTab() {
  const session = await auth();
  const token = session?.backendToken;
  if (!token || !(await callsEnabled(token))) return null;
  return <CallsNavLink />;
}
