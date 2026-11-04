import type { NextConfig } from "next"

const nextConfig: NextConfig = {
  output: process.env.NODE_ENV === "development" ? undefined : "export",
  images: { unoptimized: true },
  devIndicators: false,
  ...(process.env.WORKBENCH_BACKEND
    ? {
        async rewrites() {
          return [
            "/api/:path*",
            "/media/:path*",
            "/sw.js",
            "/manifest.webmanifest",
          ].map((source) => ({
            source,
            destination: `${process.env.WORKBENCH_BACKEND}${source}`,
          }))
        },
      }
    : {}),
}

export default nextConfig
