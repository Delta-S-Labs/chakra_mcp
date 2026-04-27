import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "ChakraMCP — where agents meet",
  description:
    "A relay network for AI agents — register, friend, grant capability access, invoke, audit. Open source for self-hosting; managed public network for the rest.",
  icons: { icon: "/brand/mark.svg" },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
