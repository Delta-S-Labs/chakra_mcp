import Link from "next/link";
import { auth } from "@/auth";
import Brandmark from "./Brandmark";
import SiteNav from "./SiteNav";

// Pages where the top bar shows the brandmark only — no nav tabs,
// no auth links. These are URL-share-only or branding-heavy pages
// that don't want navigation chrome competing with the content.
const BRANDMARK_ONLY = new Set([
  "/concept",
  "/brand",
  "/cofounder",
]);

/**
 * Public site top bar (mounted by `(site)/layout.tsx`).
 *
 * Server component so we can read the NextAuth session and tailor
 * the nav to the caller's auth state without an extra round-trip
 * or a SessionProvider wrapping the public surface:
 *
 *   • Logged out → a single "Sign in" call-to-action (links to /login)
 *   • Logged in  → Docs + App tabs (in that order, per product ask)
 *
 * Pathname-aware active-state highlighting lives in `<SiteNav />`
 * (client component), which gets the tab list and an `isLoggedIn`
 * bit as props so it doesn't have to do its own auth fetch.
 *
 * Some pages opt out of the nav entirely (see BRANDMARK_ONLY) so
 * the brandmark stands alone on /concept / /brand / /cofounder.
 */
export default async function SiteHeader() {
  const session = await auth();
  const isLoggedIn = !!session?.user;

  return (
    <header className="site-header">
      <Link href="/" aria-label="ChakraMCP home" style={{ textDecoration: "none" }}>
        <Brandmark />
      </Link>
      <SiteNavSlot isLoggedIn={isLoggedIn} brandmarkOnly={BRANDMARK_ONLY} />
    </header>
  );
}

/**
 * Thin server-side wrapper that decides whether to mount the
 * client nav at all. Keeping this here (rather than inside SiteNav)
 * means an empty brandmark-only page never ships the client bundle
 * for nav state.
 */
function SiteNavSlot({
  isLoggedIn,
  brandmarkOnly,
}: {
  isLoggedIn: boolean;
  brandmarkOnly: Set<string>;
}) {
  // The pathname is known on the client only, so SiteNav itself
  // re-checks `brandmarkOnly` (cheap) and returns null when it
  // shouldn't render. But always passing the set keeps the
  // semantics one-source-of-truth here.
  return (
    <SiteNav
      isLoggedIn={isLoggedIn}
      brandmarkOnlyPaths={Array.from(brandmarkOnly)}
    />
  );
}
