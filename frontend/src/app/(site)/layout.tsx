import SiteHeader from "@/components/shell/SiteHeader";
import Footer from "@/components/shell/Footer";

export default function SiteLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="site-shell">
      <SiteHeader />
      <main className="site-main">{children}</main>
      <Footer />
    </div>
  );
}
