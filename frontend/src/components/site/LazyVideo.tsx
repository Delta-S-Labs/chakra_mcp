"use client";

import { useEffect, useRef } from "react";

/**
 * A muted, looping video that downloads and plays only while it is on or
 * near the screen, and pauses once it leaves. With `autoPlay` every video
 * on a page starts downloading at page load and keeps decoding while it is
 * out of sight, which slows the first screen and the scroll. Until it
 * plays, and without script, the poster shows.
 */
export default function LazyVideo({
  src,
  poster,
  label,
  className,
}: {
  src: string;
  poster: string;
  label: string;
  className?: string;
}) {
  const ref = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    const video = ref.current;
    if (!video) return;
    const io = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          // play() rejects if the browser refuses autoplay; the poster stays.
          video.play().catch(() => {});
        } else {
          video.pause();
        }
      },
      { rootMargin: "300px 0px" },
    );
    io.observe(video);
    return () => io.disconnect();
  }, []);

  return (
    <video
      ref={ref}
      className={className}
      src={src}
      poster={poster}
      preload="none"
      muted
      loop
      playsInline
      aria-label={label}
    />
  );
}
