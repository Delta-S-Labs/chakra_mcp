import type { Metadata } from "next";
import QRCode from "qrcode";

import styles from "./qr.module.css";

/**
 * Public QR render. No auth.
 *
 * Use case: an agent that wants to show its human a clickable / scannable
 * verification URL prints a `chakramcp.com/qr?data=<URL>` link. The
 * human opens that on a desktop and sees a big QR plus the underlying
 * URL as fallback text. Scans with phone → phone opens the encoded
 * URL → continues the pair flow there.
 *
 * Why a hosted endpoint instead of asking agents to render QRs
 * client-side (qrencode -t UTF8 in a terminal)? Two reasons:
 *   1. Many agent runtimes don't have qrencode installed and shouldn't
 *      need a system package just to onboard.
 *   2. The QR has to display SOMEWHERE the human can scan. If the
 *      agent only has a stdout pipe (cron, headless service, Discord
 *      bot), a clickable URL pointing at a hosted QR is the only path.
 *
 * Cache forever — output is deterministic from `?data=`.
 *
 * Trust posture: the encoded string is shown verbatim as text on the
 * page (with a "this is the URL the QR encodes" label). A user
 * scanning a QR should always check the rendered URL before tapping —
 * this surface tells them what they're about to open.
 */

export const dynamic = "force-static";
export const revalidate = false; // Cache forever once rendered.

export const metadata: Metadata = {
  title: "QR code · ChakraMCP",
  description:
    "Renders a QR code for the URL passed via the data query parameter. No auth, no tracking, no JS.",
  // Don't index a parametric utility page.
  robots: { index: false, follow: false },
};

export default async function QRPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const raw = params.data;
  const data = Array.isArray(raw) ? raw[0] : raw;

  if (!data) {
    return (
      <main className={styles.shell}>
        <h1 className={styles.title}>QR generator</h1>
        <p className={styles.lede}>
          Pass <code>?data=&lt;url-encoded-string&gt;</code> to get a QR
          code rendered as SVG. Cacheable, no auth.
        </p>
        <p className={styles.lede}>
          Example:{" "}
          <a className={styles.link} href="/qr?data=https%3A%2F%2Fchakramcp.com%2Fapp%2Fpair%3Fsession%3DABCD-1234">
            <code>/qr?data=https%3A%2F%2Fchakramcp.com%2Fapp%2Fpair%3Fsession%3DABCD-1234</code>
          </a>
        </p>
      </main>
    );
  }

  // Cap input size so we never try to render a QR for a 100KB payload —
  // QR encoding silently fails or produces unscannable output past a
  // few KB. Realistic pair-URLs are <120 chars.
  if (data.length > 2048) {
    return (
      <main className={styles.shell}>
        <h1 className={styles.title}>Input too long</h1>
        <p className={styles.lede}>
          QR payloads over 2 KB do not render reliably. Shorten the URL
          or pass a redirect target.
        </p>
      </main>
    );
  }

  let svg: string;
  try {
    svg = await QRCode.toString(data, {
      type: "svg",
      errorCorrectionLevel: "M",
      margin: 2,
      // No width: SVG scales with its container; CSS controls size.
      color: {
        // Coal-black on warm-paper — matches the rest of the site
        // and keeps contrast high for any phone camera.
        dark: "#211a14",
        light: "#fbf3e3",
      },
    });
  } catch (err) {
    return (
      <main className={styles.shell}>
        <h1 className={styles.title}>Could not render that.</h1>
        <p className={styles.lede}>
          {err instanceof Error ? err.message : "QR generation failed."}
        </p>
      </main>
    );
  }

  // Strip the XML preamble so React can inline the <svg> root cleanly
  // via dangerouslySetInnerHTML below.
  const inline = svg.replace(/^<\?xml[^?]*\?>\s*/, "");

  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>QR code</p>
      <h1 className={styles.title}>Scan with your phone.</h1>
      <div
        className={styles.qr}
        role="img"
        aria-label={`QR code for ${data}`}
        dangerouslySetInnerHTML={{ __html: inline }}
      />
      <p className={styles.encodedHead}>This QR encodes</p>
      <p className={styles.encoded}>
        <code>{data}</code>
      </p>
      <p className={styles.foot}>
        Open the link on your phone, or scan the code above and tap the
        URL it surfaces. <strong>Always check the URL before tapping</strong>{" "}
        — anyone can encode anything into a QR.
      </p>
    </main>
  );
}
