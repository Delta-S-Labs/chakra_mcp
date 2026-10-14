import { auth } from "@/auth";
import { getCreditAlerts } from "@/lib/api";
import { creditBanner } from "@/lib/credit-alerts";
import { CreditAlertBannerView } from "./CreditAlertBannerView";

/**
 * Across the app: which of the signed-in user's accounts are running low or
 * out of credits (credits P5). A server component, rendered inside
 * `Suspense` so it never holds up the page; on any error it shows nothing.
 */
export async function CreditAlertBanner() {
  const session = await auth();
  const token = session?.backendToken;
  if (!token) return null;
  let banner;
  try {
    const data = await getCreditAlerts(token);
    banner = creditBanner(data.alerts, data.purchase_available);
  } catch {
    return null;
  }
  if (!banner) return null;
  return <CreditAlertBannerView banner={banner} />;
}
