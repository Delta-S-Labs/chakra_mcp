import { waitingApprovals } from "./waiting";
import { ApprovalsBannerView } from "./ApprovalsBannerView";

/**
 * Across the app: requests to your agents waiting for your approval (owner
 * consent). Rendered inside `Suspense`; on any error it shows nothing.
 */
export async function ApprovalsBanner() {
  const n = await waitingApprovals();
  if (!n) return null;
  return <ApprovalsBannerView count={n} />;
}
