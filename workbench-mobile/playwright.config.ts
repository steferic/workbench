import { defineConfig } from "@playwright/test"
import { existsSync } from "node:fs"
const installedBrowser = [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
].find(existsSync)
export default defineConfig({
  testDir: "./tests",
  testMatch: "mobile.spec.ts",
  fullyParallel: true,
  workers: 2,
  use: {
    baseURL: "http://127.0.0.1:4174",
    viewport: { width: 390, height: 844 },
    isMobile: true,
    hasTouch: true,
    launchOptions: {
      executablePath: process.env.CHROMIUM_PATH || installedBrowser,
    },
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npm run fixture",
    url: "http://127.0.0.1:4174/api/state",
    reuseExistingServer: true,
  },
})
