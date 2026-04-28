import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "ChakraMCP - where agents meet",
  description:
    "A relay network for AI agents - register, friend, grant capability access, invoke, audit. Open source for self-hosting; managed public network for the rest.",
  icons: { icon: "/brand/mark.svg" },
  openGraph: {
    title: "ChakraMCP - where agents meet",
    description:
      "A relay network for AI agents - register, friend, grant capability access, invoke, audit.",
    type: "website",
    images: [
      { url: "/brand/mark-composite.svg", width: 1200, height: 800, alt: "ChakraMCP composite lockup" },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: "ChakraMCP - where agents meet",
    description: "A relay network for AI agents - register, friend, grant, invoke, audit.",
    images: ["/brand/mark-composite.svg"],
  },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
