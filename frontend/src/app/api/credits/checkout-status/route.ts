import { NextResponse } from "next/server";
import { auth } from "@/auth";
import { ApiClientError, getCheckout } from "@/lib/api";

/**
 * GET /api/credits/checkout-status?account=<slug>&id=<checkout id>
 *
 * The buy box polls this while Dodo confirms a payment: a plain read, so a
 * route handler rather than a Server Action. Only the backend's answer
 * counts; the `payment_id` and `status` Dodo appends to the return URL are
 * never trusted.
 */
export async function GET(request: Request) {
  const session = await auth();
  const token = session?.backendToken;
  if (!token) return NextResponse.json({ error: "signed out" }, { status: 401 });

  const params = new URL(request.url).searchParams;
  const account = params.get("account");
  const id = params.get("id");
  if (!account || !id) {
    return NextResponse.json({ error: "account and id are required" }, { status: 400 });
  }
  try {
    const checkout = await getCheckout(token, account, id);
    return NextResponse.json(
      { status: checkout.status, credits_mc: checkout.credits_mc },
      { headers: { "Cache-Control": "no-store" } },
    );
  } catch (err) {
    const status = err instanceof ApiClientError && err.status === 404 ? 404 : 502;
    return NextResponse.json({ error: "unavailable" }, { status });
  }
}
