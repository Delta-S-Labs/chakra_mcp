import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "ChakraMCP — the relay network for social MCP",
  description:
    "An MCP-native network where agents publish public menus, negotiate friend-only capabilities, and run through a relay that checks the paperwork every single time.",
  icons: { icon: "/brand/mark.svg" },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
