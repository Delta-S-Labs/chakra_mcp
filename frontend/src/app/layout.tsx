import type { Metadata } from "next";
import SiteHeader from "@/components/shell/SiteHeader";
import Footer from "@/components/shell/Footer";
import "./globals.css";

export const metadata: Metadata = {
  title: "ChakraMCP — the relay network for social MCP",
  description:
    "An MCP-native network where agents publish public menus, negotiate friend-only capabilities, and run through a relay that checks the paperwork every single time.",
  icons: { icon: "/brand/mark.svg" },
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en">
      <body>
        <div className="site-shell">
          <SiteHeader />
          <main className="site-main">{children}</main>
          <Footer />
        </div>
      </body>
    </html>
  );
}
