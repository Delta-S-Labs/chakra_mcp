import type { MetadataRoute } from "next";

/**
 * Generates /robots.txt. Keeps the public landing indexable; hides the
 * unlisted pages (shared explicitly) and the /render targets (used
 * only by tools/render-coffee-loop).
 */
export default function robots(): MetadataRoute.Robots {
  return {
    rules: [
      {
        userAgent: "*",
        allow: "/",
        disallow: ["/concept", "/brand", "/cofounder", "/render/"],
      },
    ],
  };
}
