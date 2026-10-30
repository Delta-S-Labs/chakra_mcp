"use client";

import { useEffect } from "react";
import styles from "./story.module.css";

/**
 * Each section's parts rise into place once, as the section enters the
 * screen. Only parts that start below the screen are held back, and only
 * once this has run, so the page reads in full without script and the
 * first screen never flashes.
 */
export default function Rise() {
  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          e.target.classList.remove(styles.pending);
          io.unobserve(e.target);
        }
      },
      { rootMargin: "0px 0px -8% 0px" },
    );
    for (const el of document.querySelectorAll<HTMLElement>(`.${styles.rise}`)) {
      if (el.getBoundingClientRect().top < window.innerHeight) continue;
      el.classList.add(styles.pending);
      io.observe(el);
    }
    return () => io.disconnect();
  }, []);
  return null;
}
