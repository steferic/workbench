import type { Metadata, Viewport } from "next"
import { Inter } from "next/font/google"

import "./globals.css"
import "./workbench.css"
import { Providers } from "@/components/providers"

// Inter with its optical-size axis: Fluid Functionalism animates a label's
// weight together with opsz, so it gets bolder without getting wider. Without
// the axis (or with a static Inter) weight changes shift the layout.
const inter = Inter({
  subsets: ["latin"],
  axes: ["opsz"],
  variable: "--font-sans",
})

export const metadata: Metadata = {
  title: "Workbench",
  description: "Your projects and agent conversations, wherever you are.",
  appleWebApp: { capable: true, statusBarStyle: "default", title: "Workbench" },
  referrer: "no-referrer",
}

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  viewportFit: "cover",
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#f7f7f8" },
    { media: "(prefers-color-scheme: dark)", color: "#141414" },
  ],
}

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode
}>) {
  return (
    <html
      lang="en"
      suppressHydrationWarning
      className={`${inter.variable} font-sans antialiased`}
      // The accent (Blue): focus rings, the switch track and every other
      // accented detail read --fluid-accent. Change it here.
      style={{ "--fluid-accent": "#6B97FF" } as React.CSSProperties}
    >
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  )
}
