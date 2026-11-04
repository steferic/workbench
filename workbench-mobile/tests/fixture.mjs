import { createServer } from "node:http"
import { readFile } from "node:fs/promises"
import { resolve, extname } from "node:path"

// Isolated, synthetic data. This server never connects to a Workbench process.
export const projects = [
  {
    id: "workbench",
    name: "Workbench",
    path: "~/Code/workbench",
    jobs: [{ id: "review", title: "Review changes", running: false }],
    servers: [
      { port: 3000, url: "http://localhost:3000", command: "next dev" },
    ],
  },
  {
    id: "canvas",
    name: "Canvas",
    path: "~/Code/canvas",
    jobs: [],
    servers: [],
  },
  {
    id: "notes",
    name: "Field notes",
    path: "~/Code/notes",
    jobs: [],
    servers: [],
  },
].map((p) => ({ global: false, objectives: [], proposals: [], ...p }))
export const agents = [
  {
    id: "a1",
    project_id: "workbench",
    project: "Workbench",
    provider: "codex",
    model: "GPT-6",
    status: "idle",
    messages: [
      {
        role: "you",
        text: "Make the mobile project switcher simpler.",
        at: "2026-11-04T15:30:00Z",
      },
      {
        role: "tool",
        text: "Read · src/remote/page.rs",
        at: "2026-11-04T15:30:10Z",
      },
      {
        role: "agent",
        text: "The project switcher is ready. Each project has one row, and your conversation stays a tap away.\n\nI also checked the layout on narrow screens:\n\n- Project names stay readable.\n- Drafts are saved when you switch.\n- Approvals stay with their original question.\n\nYou can try it from the **Projects** button above.",
        at: "2026-11-04T15:31:00Z",
      },
    ],
  },
  {
    id: "a2",
    project_id: "workbench",
    project: "Workbench",
    provider: "claude",
    model: "Claude",
    status: "blocked",
    prompt: {
      id: "prompt-1",
      lines: ["Run the mobile regression checks?"],
      options: [
        { key: "1", label: "Yes, run the checks" },
        { key: "2", label: "No, keep reviewing" },
      ],
    },
    messages: [
      {
        role: "agent",
        text: "The changes are ready for validation.",
        at: "2026-11-04T15:32:00Z",
      },
    ],
  },
  {
    id: "b1",
    project_id: "canvas",
    project: "Canvas",
    provider: "codex",
    model: "GPT-6",
    status: "working",
    messages: [
      {
        role: "agent",
        text: "Checking the canvas keyboard controls.",
        at: "2026-11-04T15:32:00Z",
      },
    ],
  },
].map((a) => ({
  manager: false,
  queued: [],
  steps: [],
  tail: [],
  msg_epoch: "fixture",
  msg_reset: true,
  msg_total: a.messages.length,
  ...a,
}))

const root = resolve(import.meta.dirname, "..")
const types = {
  ".js": "text/javascript",
  ".css": "text/css",
  ".woff2": "font/woff2",
  ".html": "text/html",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
}
createServer(async (req, res) => {
  const url = new URL(req.url, "http://localhost")
  if (url.pathname === "/api/state") {
    res.setHeader("Content-Type", "application/json")
    res.end(
      JSON.stringify({
        instance: "fixture-1",
        at: Math.floor(Date.now() / 1000),
        projects,
        agents,
        media: [],
        desk: [],
      })
    )
    return
  }
  if (req.method === "POST") {
    for await (const chunk of req) {
      void chunk
    } // Deliberately no side effects.
    res.setHeader("Content-Type", "application/json")
    res.end(JSON.stringify({ ok: true }))
    return
  }
  try {
    const path =
      url.pathname === "/before"
        ? resolve(root, "../.taste-skill/before/index.html")
        : url.pathname.startsWith("/font/")
          ? resolve(root, "../assets/fonts", url.pathname.split("/").pop())
          : resolve(
              root,
              "out",
              "." + (url.pathname === "/" ? "/index.html" : url.pathname)
            )
    if (
      !path.startsWith(root + "/out/") &&
      !url.pathname.startsWith("/font/") &&
      url.pathname !== "/before"
    )
      throw Error()
    res.setHeader(
      "Content-Type",
      types[extname(path)] || "application/octet-stream"
    )
    res.end(await readFile(path))
  } catch {
    res.writeHead(404)
    res.end("Not found")
  }
}).listen(4174, "127.0.0.1", () =>
  console.log("Isolated mobile fixture: http://localhost:4174")
)
