import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { getMe } from "@/lib/api";
import { SurveyForm } from "./SurveyForm";
import styles from "./welcome.module.css";

/**
 * /app/welcome — first-login survey.
 *
 * The dashboard layout redirects here when /v1/me reports
 * `survey_required: true`. Once the user submits the survey, the
 * SurveyForm calls router.push("/app") and that flag flips false.
 */
export default async function WelcomePage() {
  const session = await auth();
  const token = session?.backendToken;
  if (!token) redirect("/login");

  // Defensive: if the backend says survey isn't required, just go home.
  let me;
  try {
    me = await getMe(token);
  } catch {
    redirect("/app");
  }
  if (!me.survey_required) {
    redirect("/app");
  }

  const firstName = (me.user.display_name || me.user.email).split(" ")[0];

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Welcome</div>
        <h1 className={styles.title}>Hey {firstName}. Three quick questions.</h1>
        <p className={styles.body}>
          Helps us understand who&apos;s on the network and what to ship next.
          Takes thirty seconds. Required once, never again.
        </p>
      </header>

      <SurveyForm token={token} />
    </div>
  );
}
