import CoffeeLoop from "@/components/sections/CoffeeLoop";

/**
 * Render-only route. Used by tools/render-coffee-loop to screenshot the
 * dispatch log at precise timeline points and compose an MP4/GIF.
 *
 * Not linked from the site. Lives outside the (site) route group so
 * SiteHeader/Footer don't render.
 */
export default function CoffeeLoopRenderPage() {
  return (
    <div
      style={{
        padding: "48px 40px",
        background: "var(--paper)",
        minHeight: "100vh",
        display: "grid",
        placeItems: "center",
      }}
    >
      <div style={{ width: "min(1120px, 100%)" }}>
        <CoffeeLoop />
      </div>
    </div>
  );
}
