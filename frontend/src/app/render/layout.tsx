import type { Metadata } from "next";

/**
 * The /render/* routes are used by the offline capture tool in
 * tools/render-coffee-loop. They should never appear in search or
 * be linked from the site.
 */
export const metadata: Metadata = {
  robots: { index: false, follow: false },
};

export default function RenderLayout({ children }: { children: React.ReactNode }) {
  return <>{children}</>;
}
