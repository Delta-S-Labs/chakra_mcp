import { NextResponse } from "next/server";

/**
 * POST /api/captcha/verify
 *
 * Body: { token: string }
 *
 * Verifies a Google reCAPTCHA v2 token against the siteverify endpoint.
 * Server-side only - the secret never touches the browser. Gated by
 * CAPTCHA_ENABLED so private deployments can no-op this entirely.
 */

export async function POST(request: Request) {
  if (process.env.CAPTCHA_ENABLED === "false") {
    return NextResponse.json({ ok: true, skipped: true });
  }

  const secret = process.env.RECAPTCHA_SECRET_KEY;
  if (!secret) {
    return NextResponse.json(
      { ok: false, error: "Captcha is enabled but RECAPTCHA_SECRET_KEY is not set." },
      { status: 500 },
    );
  }

  let token: string | undefined;
  try {
    const body = (await request.json()) as { token?: string };
    token = body.token;
  } catch {
    return NextResponse.json({ ok: false, error: "Invalid JSON." }, { status: 400 });
  }

  if (!token) {
    return NextResponse.json({ ok: false, error: "Missing captcha token." }, { status: 400 });
  }

  const res = await fetch("https://www.google.com/recaptcha/api/siteverify", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ secret, response: token }).toString(),
  });

  const data = (await res.json()) as { success: boolean; "error-codes"?: string[] };

  if (!data.success) {
    return NextResponse.json(
      { ok: false, error: "Captcha failed verification.", details: data["error-codes"] },
      { status: 400 },
    );
  }

  return NextResponse.json({ ok: true });
}
