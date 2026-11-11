# This project

A video edit as a React + TypeScript project, rendered by raea.

- edit.tsx: the composition. Change anything: layout, timing, which component plays when.
- cuts.ts: the clips (`raea cut` writes it, with cuts.json and words.json beside it; re-run it rather than editing
  by hand, or captions and checks lose their timing). components/: parts you added (`raea part add`), yours to change.
- brand/brand.css: the brand's tokens (`raea brand DOMAIN` reads them from the brand's site). edit.tsx loads them with the project's
  Tailwind: `<style>{css}</style>`, css from "./styles.gen" (raea render writes it). Style only with the brand
  classes: colour text-brand-accent, bg-brand-accent, bg-brand-surface (bg-brand-bg), text-brand-on-surface,
  text-brand-on-accent, text-brand-ink, brand-1/2/3; face font-brand, font-brand-body; size text-display,
  text-title, text-caption, text-body, text-label (scaled for a 1080-wide frame); corners rounded-brand. Where a
  class can't reach, the --brand-* variables. Never a colour, font or size of your own.
- `raea render edit.tsx -o final.mp4 --manifest render.json`, then `raea check final.mp4 --manifest render.json
  --words words.json --perception perception.json` says what's wrong; `raea render edit.tsx --at 105 -o stills/`
  gives frame 105 to look at. `raea guide` has the craft (`raea guide motion` for writing the TSX);
  `raea help` lists the rest.

## raea (everything there is; import from "raea")

Time: useCurrentFrame() (frames, from 0 inside the nearest Sequence), useVideoConfig() {width, height, fps,
durationInFrames}, interpolate(frame, [in...], [out...], {extrapolateLeft, extrapolateRight: "clamp", easing}),
spring({frame, fps, config}), Easing.bezier(a, b, c, d) / Easing.inOut(...).
Layout: <AbsoluteFill>, <Sequence from durationInFrames>, <Series><Series.Sequence durationInFrames>, <Loop>, <Freeze>.
Media: <Video src startFrom endAt volume playbackRate muted framing="face" punchIn style> (framing="face" keeps the
eyes 38% down as the speaker moves; an objectPosition or transform turns it off), <Audio src startFrom endAt volume={n | (f) => n} gainDb fadeIn fadeOut>,
<Img src>. Media files import as modules: import take from
"./media/x.mov". Sound: music in Tone.js (import * as Tone from "raea/tone"): instruments in a Tone.Sampler
playing a Part or Sequence, the instrument's notes from Tone.tokensynth("a warm upright bass", ["C2", "C3", "C4"]) or a
sample library (`raea sounds`); sao("prompt", {seconds}) for an effect, as an <Audio src>; fish("words") for
narration. All local but fish. <Mix lufs ceiling> masters the sound.
Words: useWords() -> [{text, from, to}] (frames, the cuts' words), word("phrase") -> the frame it's said,
findPhrase(words, "phrase", {occurrence, edge: "start" | "end"}).
Placement: <Region name="top" | "bottom" | "center" | "auto" | "full"> keeps a part in a band clear of the face and
the platforms' buttons; useRegion() gives its box.
Composition: registerRoot(() => <Composition id component width height fps durationInFrames />).
Not there: React hooks come from "react" (useMemo, useState), not from raea.
