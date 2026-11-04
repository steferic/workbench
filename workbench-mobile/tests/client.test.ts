import { test } from "node:test"
import assert from "node:assert/strict"
import { WorkbenchClient } from "../lib/workbench/client"
import type { Snapshot, Agent, Project } from "../lib/workbench/types"

const a = (id: string, project = "p"): Agent => ({
  id,
  project_id: project,
  project,
  provider: "codex",
  status: "idle",
  queued: [],
  steps: [],
  messages: [],
  tail: [],
  msg_reset: true,
  msg_total: 0,
  msg_epoch: "one",
})
const p = (id: string): Project => ({
  id,
  name: id,
  path: id,
  jobs: [],
  objectives: [],
  proposals: [],
})
const snapshot = (): Snapshot => ({
  instance: "desktop-1",
  at: Math.floor(Date.now() / 1000),
  agents: [a("A"), a("B", "q")],
  projects: [p("p"), p("q"), p("empty")],
  media: [],
  desk: [],
})
const response = (body: unknown = { ok: true }, status = 200) =>
  new Response(typeof body === "string" ? body : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
const tick = () => new Promise((resolve) => setImmediate(resolve))
function setup(
  fetcher: typeof fetch = async () => response(),
  saved: Record<string, string> = {}
) {
  const storage = {
    getItem: (k: string) => saved[k] || null,
    setItem: (k: string, v: string) => {
      saved[k] = v
    },
  }
  const c = new WorkbenchClient("test", storage, fetcher)
  c.state = {
    ...c.state,
    data: snapshot(),
    current: "A",
    project: "p",
    connection: "online",
  }
  return c
}

test("failed send keeps draft; retry keeps command identity and clears only after ack", async () => {
  const requests: RequestInit[] = []
  let fail = true
  const c = setup(async (_, init) => {
    requests.push(init!)
    return fail ? response("outage", 503) : response()
  })
  c.setText("keep this draft")
  await c.submit()
  assert.equal(c.draft().text, "keep this draft")
  assert.equal(c.state.outbox[0].status, "unknown")
  fail = false
  await c.deliver(c.state.outbox[0].id)
  assert.equal(
    new Headers(requests[0].headers).get("X-Command-Id"),
    new Headers(requests[1].headers).get("X-Command-Id")
  )
  assert.equal(c.draft().text, "")
  assert.equal(c.state.outbox[0].status, "accepted")
})
test("double taps send once; acknowledgment cannot erase a newer draft", async () => {
  let finish!: (r: Response) => void
  let count = 0
  const c = setup(() => {
    count++
    return new Promise((resolve) => {
      finish = resolve
    })
  })
  c.setText("first")
  const sending = c.submit()
  void c.submit()
  assert.equal(count, 1)
  c.setText("newer draft")
  finish(response())
  await sending
  assert.equal(c.draft().text, "newer draft")
})
test("drafts and unconfirmed requests survive reload, retaining the instance and ID", async () => {
  const saved = {}
  const c = setup(async () => response("offline", 503), saved)
  c.setText("survive reload")
  await c.submit()
  const next = setup(undefined, saved)
  assert.equal(next.draft().text, "survive reload")
  assert.equal(next.state.outbox[0].id, c.state.outbox[0].id)
  assert.equal(next.state.outbox[0].instance, "desktop-1")
})
test("each delayed upload stays with the originating agent and blocks partial sends", async () => {
  let finish!: (r: Response) => void
  const uploads: string[] = []
  const c = setup(async (url, init) => {
    if (String(url).includes("/api/state")) return response(snapshot())
    assert.ok(init?.method === "POST")
    uploads.push(String(url))
    if (uploads.length === 1)
      return new Promise((resolve) => {
        finish = resolve
      })
    return response({ path: "/uploads/A/two.txt" })
  })
  c.setText("caption")
  const upload = c.upload([
    new File(["one"], "one.txt"),
    new File(["two"], "two.txt"),
  ])
  await c.submit()
  assert.equal(c.state.outbox.length, 0)
  c.selectAgent("B")
  finish(response({ path: "/uploads/A/one.txt" }))
  await upload
  assert.equal(c.draft("A").attached.length, 2)
  assert.equal(c.draft("B").attached.length, 0)
  assert.ok(uploads.every((url) => url.includes("agent=A")))
})
test("projects with no agents remain selected and can start their own conversation", async () => {
  const c = setup(async () => response(snapshot()))
  c.selectProject("empty")
  await tick()
  await c.refresh()
  assert.equal(c.state.project, "empty")
  assert.equal(c.state.current, null)
  assert.equal(c.state.thread.length, 0)
  const data = snapshot()
  data.agents.push(a("C", "empty"))
  const d = setup(async () => response(data))
  d.selectProject("empty")
  await tick()
  assert.equal(d.state.current, "C")
  assert.equal(d.state.project, "empty")
})
test("switching projects restores each project’s last selected agent and draft", async () => {
  const data = snapshot()
  data.agents.push(a("A2"))
  const c = setup(async () => response(data))
  c.state.data = data
  c.selectAgent("A2")
  c.setText("A2 draft")
  c.selectProject("q")
  c.setText("B draft")
  c.selectProject("p")
  await tick()
  assert.equal(c.state.current, "A2")
  assert.equal(c.draft().text, "A2 draft")
  assert.equal(c.draft("B").text, "B draft")
})
test("the restored initial agent is remembered even without an explicit agent pick", async () => {
  const data = snapshot()
  data.agents.push({ ...a("A2"), status: "blocked" })
  const c = setup(async () => response(data))
  await c.refresh()
  c.setText("my draft")
  c.selectProject("q")
  await tick()
  c.selectProject("p")
  await tick()
  assert.equal(c.state.current, "A")
  assert.equal(c.draft().text, "my draft")
})
test("history is bounded, replacement text updates, and epochs reset server cursors", async () => {
  const data = snapshot()
  const c = setup(async () => response(data))
  data.agents[0].messages = Array.from({ length: 300 }, (_, i) => ({
    role: "agent",
    text: String(i),
  }))
  data.agents[0].msg_total = 300
  await c.refresh()
  assert.equal(c.state.thread.length, 200)
  assert.equal(c.state.have, 300)
  data.agents[0].tail = ["new"]
  data.agents[0].messages = [{ role: "agent", text: "replacement" }]
  data.agents[0].msg_total = 1
  await c.refresh()
  assert.equal(c.state.thread[0].text, "replacement")
  assert.equal(c.agent?.tail[0], "new")
  data.agents[0].msg_epoch = "two"
  data.agents[0].messages = []
  data.agents[0].msg_total = 0
  await c.refresh()
  assert.equal(c.state.have, 0)
  assert.deepEqual(c.state.thread, [])
})
test("an old agent response cannot advance the new agent cursor", async () => {
  let finish!: (r: Response) => void
  const c = setup(
    () =>
      new Promise((resolve) => {
        finish = resolve
      })
  )
  const pending = c.refresh()
  const originalFinish = finish
  c.selectAgent("B")
  const old = snapshot()
  old.agents[0].messages = [{ role: "agent", text: "old" }]
  old.agents[0].msg_total = 1
  originalFinish(response(old))
  await pending
  await tick()
  assert.equal(c.state.current, "B")
  assert.equal(c.state.have, 0)
  assert.deepEqual(c.state.thread, [])
  finish(response(snapshot()))
  await tick()
})
test("status polling continues while a write waits for acknowledgment", async () => {
  let finish!: (r: Response) => void
  const data = snapshot()
  data.agents[0].tail = ["still updating"]
  const c = setup(async (_, init) =>
    init?.method === "POST"
      ? new Promise((resolve) => {
          finish = resolve
        })
      : response(data)
  )
  c.setText("hello")
  const pending = c.submit()
  await c.refresh()
  assert.equal(c.agent?.tail[0], "still updating")
  finish(response())
  await pending
})
test("stalled requests abort at their deadline", async () => {
  const c = setup(
    (_, options) =>
      new Promise((_, reject) =>
        options?.signal?.addEventListener("abort", () =>
          reject(Error("aborted"))
        )
      )
  )
  await assert.rejects(c.request("/api/state", {}, 5), /aborted/)
})
test("answers require current question identity and carry the ticket to the server", async () => {
  const requests: RequestInit[] = []
  const data = snapshot()
  data.agents[0].prompt = {
    id: "question-2",
    lines: [],
    options: [{ key: "1", label: "yes" }],
  }
  const c = setup(async (_, options) => {
    if (options?.method === "POST") requests.push(options)
    return options?.method === "POST" ? response() : response(data)
  })
  c.state.data = data
  await c.answer("1", "question-1", "A")
  assert.equal(requests.length, 0)
  await c.answer("1", "question-2", "A")
  assert.equal(
    new Headers(requests[0].headers).get("X-Prompt-Id"),
    "question-2"
  )
})
test("ambiguous action retries retain identity, while stale questions can be retried fresh", async () => {
  const ids: string[] = []
  let status = 503
  const c = setup(async (_, options) => {
    ids.push(new Headers(options?.headers).get("X-Command-Id")!)
    return response("failure", status)
  })
  await c.action("/api/job", { agent: "p", text: "review" })
  await c.action("/api/job", { agent: "p", text: "review" })
  assert.equal(ids[0], ids[1])
  status = 409
  await c.action("/api/job", { agent: "p", text: "review" })
  await c.action("/api/job", { agent: "p", text: "review" })
  assert.notEqual(ids[2], ids[3])
})

const remoteSnapshot = (): Snapshot => {
  const data = snapshot()
  data.machines = [{ name: "mbp6", status: "online", instance: "remote-1" }]
  data.projects.push({ ...p("mbp6~p"), name: "p @ mbp6" })
  data.agents.push(a("mbp6~A", "mbp6~p"))
  return data
}
test("remote commands keep their computer, agent, and process identity across retries", async () => {
  const saved = {}
  const requests: RequestInit[] = []
  const data = remoteSnapshot()
  let fail = true
  const c = setup(async (url, init) => {
    if (String(url).includes("/api/state")) return response(data)
    requests.push(init!)
    return fail
      ? response("Unconfirmed", 502)
      : response("The remote computer restarted", 409)
  }, saved)
  c.state.data = data
  c.selectAgent("mbp6~A")
  await tick()
  c.setText("Only for the other computer")
  await c.submit()
  assert.equal(JSON.parse(requests[0].body as string).agent, "mbp6~A")
  const original = new Headers(requests[0].headers)
  assert.equal(original.get("X-Peer-Instance"), "remote-1")
  assert.equal(original.get("X-Workbench-Instance"), "desktop-1")
  assert.equal(c.state.outbox[0].status, "unknown")
  const restored = setup(undefined, saved)
  assert.equal(restored.state.outbox[0].peerInstance, "remote-1")
  data.machines![0].instance = "remote-2"
  fail = false
  await c.deliver(c.state.outbox[0].id)
  assert.equal(
    new Headers(requests[1].headers).get("X-Peer-Instance"),
    "remote-1"
  )
  assert.equal(
    new Headers(requests[1].headers).get("X-Command-Id"),
    original.get("X-Command-Id")
  )
  assert.equal(c.state.outbox[0].status, "failed")
  assert.equal(c.draft("mbp6~A").text, "Only for the other computer")
  assert.equal(c.draft("A").text, "")
})
test("offline remote projects keep drafts and prevent writes until they reconnect", async () => {
  const data = remoteSnapshot()
  data.machines![0].status = "offline"
  let writes = 0
  const c = setup(async (url) => {
    if (String(url).includes("/api/state")) return response(data)
    writes++
    return response()
  })
  c.state.data = data
  c.selectProject("mbp6~p")
  await tick()
  c.setText("Wait for this computer")
  await c.submit()
  await c.upload([new File(["hello"], "hello.txt")])
  await c.action("/api/new-agent", { agent: "mbp6~p", text: "codex" })
  assert.equal(writes, 0)
  assert.equal(c.state.project, "mbp6~p")
  assert.equal(c.draft().text, "Wait for this computer")
  data.machines![0].status = "online"
  await c.refresh()
  assert.equal(writes, 0, "reconnection never sends automatically")
  await c.submit()
  assert.equal(writes, 1)
  assert.equal(c.draft().text, "")
})
test("a remote decision is addressed by its own computer while a local agent is selected", async () => {
  const requests: RequestInit[] = []
  const data = remoteSnapshot()
  const c = setup(async (url, init) => {
    if (String(url).includes("/api/state")) return response(data)
    requests.push(init!)
    return response()
  })
  c.state.data = data
  await c.action("/api/proposal", { agent: "mbp6~proposal", text: "approve" })
  assert.equal(
    new Headers(requests[0].headers).get("X-Peer-Instance"),
    "remote-1"
  )
  assert.equal(JSON.parse(requests[0].body as string).agent, "mbp6~proposal")
})
