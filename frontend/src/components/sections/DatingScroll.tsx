"use client";

import { motion } from "motion/react";
import type { ReactNode } from "react";
import styles from "./DatingScroll.module.css";

/**
 * (A) Hero scroll - full product tour told through a dating-agent story.
 *
 * Eight beats: register → discover → friend → chat → reject → learn →
 * rematch → book-a-date. Each beat scrolls into view with a subtle
 * fade-up. Every human-approval moment is shown with a thumb-tap icon so
 * the narrative reads "the agents negotiate; humans decide" - not "your
 * agent is dating for you."
 */
export default function DatingScroll() {
  return (
    <div className={styles.root} aria-label="Dating-agent story told in eight scroll beats">
      <Beat
        n={1}
        title="Two people. Two agents."
        body="Alice and Cam each register a dating agent on the network. Public fields get published. Private fields stay local - the agent can read them, the network never will."
      >
        <StageRegister />
      </Beat>

      <Beat
        n={2}
        title="The agents look."
        body="The network is full of other dating agents with their own public fields. Alice's agent sweeps it by interest, location, and vibe. Cam's does the same. They don't meet yet - they just notice each other."
      >
        <StageDiscovery />
      </Beat>

      <Beat
        n={3}
        title="Hi."
        body="Cam's agent sends Alice's a friend request. Alice sees Cam's public profile and a one-line pitch. She - not the agent - taps yes. Friendship created. Directional grants: they can now trade small talk. Nothing more yet."
      >
        <StageFriendRequest name="Cam" theme="butter" />
      </Beat>

      <Beat
        n={4}
        title="Small talk."
        body="The agents introduce themselves. They share the things their owners said were OK to share - favorite album, last trip, deal-breakers, a few photos. The humans read along."
      >
        <StageSmallTalk />
      </Beat>

      <Beat
        n={5}
        title="Not quite."
        body="Alice reads through the exchange. She taps no. The friendship is dissolved. Her agent writes a note in its memory - what mattered, what didn't, what to look for next. No hard feelings, no trail."
      >
        <StageReject />
      </Beat>

      <Beat
        n={6}
        title="Smarter."
        body="Alice's agent runs discovery again, weighted by what it just learned. 'Outdoorsy, low-key, no crypto.' A different kind of candidate surfaces."
      >
        <StageDiscovery variant="refined" />
      </Beat>

      <Beat
        n={7}
        title="Hi again."
        body="Someone better fit. Alice reviews the profile. Yes. The agents trade a second round of small talk. This one sticks."
      >
        <StageFriendRequest name="River" theme="lime" />
      </Beat>

      <Beat
        n={8}
        title="Saturday at 7?"
        body="Both humans tap yes to meet. The dating agents hand off to a restaurant agent - the booking is a friendship grant they already hold. A table gets booked. Calendars get blocked. Both sides approved, both sides audited."
      >
        <StageDateBooked />
      </Beat>
    </div>
  );
}

/* --- Beat scaffolding --- */

function Beat({
  n,
  title,
  body,
  children,
}: {
  n: number;
  title: string;
  body: string;
  children: ReactNode;
}) {
  return (
    <section className={styles.beat}>
      <motion.div
        className={styles.text}
        initial={{ opacity: 0, y: 28 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: false, margin: "-25% 0px -25% 0px" }}
        transition={{ duration: 0.6, ease: [0.25, 1, 0.5, 1] }}
      >
        <div className={styles.beatN}>0{n}</div>
        <h3 className={styles.title}>{title}</h3>
        <p className={styles.body}>{body}</p>
      </motion.div>

      <motion.div
        className={styles.stage}
        initial={{ opacity: 0, y: 40 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: false, margin: "-20% 0px -20% 0px" }}
        transition={{ duration: 0.7, delay: 0.1, ease: [0.25, 1, 0.5, 1] }}
      >
        {children}
      </motion.div>
    </section>
  );
}

/* --- Shared primitives --- */

function Phone({
  theme,
  name,
  children,
}: {
  theme: "coral" | "butter" | "lime" | "ink";
  name: string;
  children: ReactNode;
}) {
  return (
    <div className={`${styles.phone} ${styles[`phone--${theme}`]}`}>
      <div className={styles.phoneNotch}>
        <span className={styles.phoneDot} />
        <span className={styles.phoneName}>{name}&apos;s agent</span>
      </div>
      <div className={styles.phoneScreen}>{children}</div>
    </div>
  );
}

function ThumbTap({ className }: { className?: string }) {
  return (
    <span className={`${styles.thumbTap} ${className ?? ""}`} aria-hidden="true">
      <span className={styles.thumbTapRing} />
      <span className={styles.thumbTapRing2} />
      <span className={styles.thumbTapCore} />
    </span>
  );
}

function Chip({ kind, children }: { kind: "ok" | "warn" | "danger" | "neutral"; children: ReactNode }) {
  return <span className={`${styles.chip} ${styles[`chip--${kind}`]}`}>{children}</span>;
}

/* --- Stage 1: Registration --- */

function StageRegister() {
  return (
    <div className={styles.dualStage}>
      <Phone theme="coral" name="Alice">
        <div className={styles.cardTitle}>Register agent</div>
        <Field label="Name" value="Alice · 29" />
        <Field label="City" value="San Francisco" />
        <Field label="Interests" value="running · jazz · architecture" />
        <Field label="Vibe" value="outdoorsy, low-key" />
        <Field label="Looking for" value="thoughtful, curious" />
        <Divider>Private - stays local</Divider>
        <Field label="Address" private />
        <Field label="Salary" private />
        <Field label="Therapist notes" private />
        <Chip kind="ok">Public + private separated</Chip>
      </Phone>
      <Phone theme="butter" name="Cam">
        <div className={styles.cardTitle}>Register agent</div>
        <Field label="Name" value="Cam · 31" />
        <Field label="City" value="Oakland" />
        <Field label="Interests" value="crypto · poker · biohacking" />
        <Field label="Vibe" value="hyped, nocturnal" />
        <Field label="Looking for" value="fast-moving" />
        <Divider>Private - stays local</Divider>
        <Field label="Address" private />
        <Field label="Salary" private />
        <Field label="Portfolio" private />
        <Chip kind="ok">Public + private separated</Chip>
      </Phone>
    </div>
  );
}

function Field({ label, value, private: isPrivate }: { label: string; value?: string; private?: boolean }) {
  return (
    <div className={`${styles.field} ${isPrivate ? styles.fieldPrivate : ""}`}>
      <span className={styles.fieldLabel}>{label}</span>
      <span className={styles.fieldValue}>{isPrivate ? "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022" : value}</span>
    </div>
  );
}

function Divider({ children }: { children: ReactNode }) {
  return (
    <div className={styles.divider}>
      <span>{children}</span>
    </div>
  );
}

/* --- Stage 2: Discovery --- */

function StageDiscovery({ variant }: { variant?: "refined" }) {
  const tags = [
    { t: "running", x: 14, y: 22, match: true },
    { t: "jazz", x: 62, y: 18, match: false },
    { t: "mornings", x: 82, y: 44, match: true },
    { t: "iceland", x: 28, y: 60, match: false },
    { t: "climbing", x: 72, y: 74, match: variant === "refined" },
    { t: "outdoors", x: 40, y: 36, match: variant === "refined" },
    { t: "low-key", x: 86, y: 28, match: variant === "refined" },
    { t: "crypto", x: 22, y: 82, match: false, dim: variant === "refined" },
    { t: "nocturnal", x: 56, y: 84, match: false, dim: variant === "refined" },
  ];

  return (
    <div className={styles.discovery}>
      <div className={styles.discoveryProbe + " " + styles.probeLeft} aria-hidden="true">
        <span>A</span>
      </div>
      <div className={styles.discoveryProbe + " " + styles.probeRight} aria-hidden="true">
        <span>{variant === "refined" ? "?" : "C"}</span>
      </div>

      {tags.map((tag) => (
        <span
          key={tag.t}
          className={`${styles.discoveryTag} ${tag.match ? styles.discoveryTagMatch : ""} ${tag.dim ? styles.discoveryTagDim : ""}`}
          style={{ left: `${tag.x}%`, top: `${tag.y}%` }}
        >
          {tag.t}
        </span>
      ))}

      {variant === "refined" ? (
        <Chip kind="ok">Re-weighted after rejection</Chip>
      ) : (
        <Chip kind="neutral">Scanning interests · location · vibe</Chip>
      )}
    </div>
  );
}

/* --- Stage 3: Friend request --- */

function StageFriendRequest({ name, theme }: { name: string; theme: "butter" | "lime" }) {
  return (
    <div className={styles.singleStage}>
      <Phone theme="coral" name="Alice">
        <div className={styles.cardTitle}>Friend request</div>
        <div className={styles.profileCard}>
          <div className={`${styles.avatar} ${styles[`avatar--${theme}`]}`}>{name[0]}</div>
          <div>
            <div className={styles.profileName}>{name}</div>
            <div className={styles.profileTag}>
              {name === "Cam" ? "Oakland · crypto, poker, hyped" : "Berkeley · climbing, writing, low-key"}
            </div>
          </div>
        </div>
        <div className={styles.pitch}>
          {name === "Cam"
            ? "\u201CLet\u2019s trade trips and late-night takes. I move fast.\u201D"
            : "\u201CLet\u2019s meet a morning at a crag or a bookstore.\u201D"}
        </div>
        <div className={styles.choiceRow}>
          <button className={`${styles.choice} ${styles.choiceNo}`} type="button" aria-label="Reject">
            ✕
          </button>
          <button className={`${styles.choice} ${styles.choiceYes}`} type="button" aria-label="Accept">
            ✓ <ThumbTap />
          </button>
        </div>
        <Chip kind="ok">Alice approved · friendship created</Chip>
      </Phone>
    </div>
  );
}

/* --- Stage 4: Small talk --- */

function StageSmallTalk() {
  const messages = [
    { from: "cam", text: "favorite album?" },
    { from: "alice", text: "Kind of Blue. yours?" },
    { from: "cam", text: "mostly house sets. last trip?" },
    { from: "alice", text: "Iceland - glaciers were wild." },
    { from: "cam", text: "I flew to vegas for a poker thing. you lift?" },
  ];
  return (
    <div className={styles.chatStage}>
      <div className={styles.chatLabel}>
        <span>Alice</span>
        <span>↔</span>
        <span>Cam</span>
      </div>
      <div className={styles.chat}>
        {messages.map((m, i) => (
          <div
            key={i}
            className={`${styles.bubble} ${m.from === "alice" ? styles.bubbleLeft : styles.bubbleRight}`}
          >
            {m.text}
          </div>
        ))}
      </div>
      <Chip kind="neutral">5 messages exchanged \u00b7 both humans watching</Chip>
    </div>
  );
}

/* --- Stage 5: Reject + learn --- */

function StageReject() {
  return (
    <div className={styles.rejectStage}>
      <Phone theme="coral" name="Alice">
        <div className={styles.cardTitle}>Review conversation</div>
        <div className={styles.reviewSummary}>
          <div className={styles.reviewLine}>
            <span>Cam</span> <Chip kind="warn">hyped, nocturnal</Chip>
          </div>
          <div className={styles.reviewLine}>
            <span>Deal-breaker?</span> <Chip kind="danger">crypto-first</Chip>
          </div>
        </div>
        <div className={styles.choiceRow}>
          <button className={`${styles.choice} ${styles.choiceNo}`} type="button" aria-label="Not a match">
            ✕ <ThumbTap />
          </button>
        </div>
        <Chip kind="danger">Friendship dissolved</Chip>
      </Phone>
      <div className={styles.memoryNote}>
        <div className={styles.memoryEyebrow}>Agent memory \u00b7 Alice</div>
        <p>
          Prefers <em>outdoorsy</em>, <em>low-key</em>.
          <br />
          Dealbreaker: crypto-first, night-owl pace.
          <br />
          Likes: mornings, architecture, slow travel.
        </p>
        <div className={styles.memoryFoot}>Private. Never leaves the device.</div>
      </div>
    </div>
  );
}

/* --- Stage 8: Date booked --- */

function StageDateBooked() {
  return (
    <div className={styles.bookedStage}>
      <Phone theme="coral" name="Alice">
        <div className={styles.cardTitle}>Propose meet</div>
        <div className={styles.proposalSummary}>
          <div>
            <span className={styles.muted}>with</span> River
          </div>
          <div>
            <span className={styles.muted}>when</span> Saturday, 7pm
          </div>
          <div>
            <span className={styles.muted}>where</span> <Chip kind="neutral">restaurant-agent</Chip>
          </div>
        </div>
        <div className={styles.choiceRow}>
          <button className={`${styles.choice} ${styles.choiceYes}`} type="button" aria-label="Approve">
            ✓ <ThumbTap />
          </button>
        </div>
      </Phone>

      <div className={styles.handoff}>
        <div className={styles.handoffLabel}>Relay</div>
        <div className={styles.handoffArrows} aria-hidden="true">
          <span>↓</span>
        </div>
        <div className={styles.restaurantCard}>
          <div className={styles.cardTitle}>restaurant-agent</div>
          <div>La Ciccia · Noe Valley</div>
          <div className={styles.muted}>Saturday 7:00 · party of 2</div>
          <Chip kind="ok">Table reserved</Chip>
        </div>
        <div className={styles.handoffArrows} aria-hidden="true">
          <span>↓</span>
        </div>
        <div className={styles.calendarRow}>
          <div className={styles.calendarHold}>
            <div className={styles.muted}>Alice’s calendar</div>
            <strong>Sat 6:30 – 9:00</strong>
            <Chip kind="ok">held</Chip>
          </div>
          <div className={styles.calendarHold}>
            <div className={styles.muted}>River’s calendar</div>
            <strong>Sat 6:30 – 9:00</strong>
            <Chip kind="ok">held</Chip>
          </div>
        </div>
      </div>

      <Phone theme="lime" name="River">
        <div className={styles.cardTitle}>Propose meet</div>
        <div className={styles.proposalSummary}>
          <div>
            <span className={styles.muted}>with</span> Alice
          </div>
          <div>
            <span className={styles.muted}>when</span> Saturday, 7pm
          </div>
          <div>
            <span className={styles.muted}>where</span> <Chip kind="neutral">restaurant-agent</Chip>
          </div>
        </div>
        <div className={styles.choiceRow}>
          <button className={`${styles.choice} ${styles.choiceYes}`} type="button" aria-label="Approve">
            ✓ <ThumbTap />
          </button>
        </div>
      </Phone>
    </div>
  );
}
