"use client";

import { useEffect } from "react";

/* Marks the page as scrolled so the floating bar can shrink (site.css
   reads `html[data-scrolled]`). Renders nothing. */
export default function HeaderScroll() {
  useEffect(() => {
    const root = document.documentElement;
    const update = () => {
      if (window.scrollY > 24) root.dataset.scrolled = "";
      else delete root.dataset.scrolled;
    };
    update();
    window.addEventListener("scroll", update, { passive: true });
    return () => {
      window.removeEventListener("scroll", update);
      delete root.dataset.scrolled;
    };
  }, []);
  return null;
}
