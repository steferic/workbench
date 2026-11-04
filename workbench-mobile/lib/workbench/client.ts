import type {
  Agent,
  ClientState,
  CommandMeta,
  Delivery,
  Draft,
  Snapshot,
} from "./types"

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">
export class RequestError extends Error {
  constructor(
    message: string,
    public status = 0
  ) {
    super(message)
  }
}
const blankDraft = (): Draft => ({ text: "", attached: [], revision: 0 })
export const agentName = (a: Agent) => a.alias || a.model || a.provider

// The controller owns delivery, cursors, and drafts independently of React.
// A menu or conversation unmount must never cancel an acknowledged command.
export class WorkbenchClient {
  state: ClientState
  private listeners = new Set<() => void>()
  private tag: string | null = null
  private clockOffset = 0
  private refreshing = false
  private actions = new Map<string, CommandMeta & { pending: boolean }>()
  private lastAgent: Record<string, string>

  constructor(
    public token: string,
    private storage?: Storage,
    private fetcher: typeof fetch = (input, init) => fetch(input, init)
  ) {
    this.lastAgent = this.saved("project-agents", {})
    this.state = {
      data: null,
      current: this.read("agent"),
      project: this.read("project"),
      thread: [],
      have: 0,
      epoch: "",
      drafts: this.saved("drafts", {}),
      outbox: this.saved<Delivery[]>("outbox", []).map((item) => ({
        ...item,
        inflight: false,
        status: item.status === "pending" ? "unknown" : item.status,
      })),
      connection: "connecting",
      notice: "",
      pendingActions: [],
      uploading: {},
    }
  }
  private read(key: string) {
    try {
      return this.storage?.getItem(key) || null
    } catch {
      return null
    }
  }
  private saved<T>(key: string, fallback: T): T {
    try {
      return JSON.parse(this.read(key) || "null") ?? fallback
    } catch {
      return fallback
    }
  }
  private save(key: string, value: unknown) {
    try {
      this.storage?.setItem(
        key,
        typeof value === "string"
          ? value
          : JSON.stringify(value, (key, value) =>
              key === "preview" || key === "inflight" ? undefined : value
            )
      )
    } catch {
      /* In-memory recovery survives storage restrictions. */
    }
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }
  getSnapshot = () => this.state
  private emit(patch: Partial<ClientState> = {}) {
    this.state = { ...this.state, ...patch }
    this.listeners.forEach((listener) => listener())
  }
  notice = (notice: string) => this.emit({ notice })
  url = (path: string) =>
    path +
    (path.includes("?") ? "&" : "?") +
    "t=" +
    encodeURIComponent(this.token)
  get agent() {
    return this.state.data?.agents.find((a) => a.id === this.state.current)
  }
  draft(id = this.state.current): Draft {
    return id ? this.state.drafts[id] || blankDraft() : blankDraft()
  }
  private writeDraft(id: string, draft: Draft) {
    this.state.drafts = { ...this.state.drafts, [id]: draft }
    this.save("drafts", this.state.drafts)
    this.emit()
  }
  setText(text: string, owner = this.state.current) {
    if (owner) {
      const d = this.draft(owner)
      this.writeDraft(owner, { ...d, text, revision: d.revision + 1 })
    }
  }
  private persistOutbox() {
    this.save("outbox", this.state.outbox)
  }
  machine(id = this.state.current) {
    return this.state.data?.machines?.find((m) => id?.startsWith(m.name + "~"))
  }
  available(id = this.state.current) {
    const machine = this.machine(id)
    return !machine || (machine.status === "online" && !!machine.instance)
  }
  private requireAvailable(id: string | null) {
    if (this.available(id)) return true
    this.notice(
      `${this.machine(id)?.name} is unavailable. Your draft is saved; try when it reconnects.`
    )
    return false
  }
  meta(target = this.state.current): CommandMeta {
    // randomUUID isn't exposed on plain tailnet HTTP; getRandomValues is.
    const bytes = crypto.getRandomValues(new Uint8Array(16))
    bytes[6] = (bytes[6] & 15) | 64
    bytes[8] = (bytes[8] & 63) | 128
    const h = [...bytes].map((x) => x.toString(16).padStart(2, "0")).join("")
    return {
      id: `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`,
      issued: Math.floor(Date.now() / 1000) + this.clockOffset,
      instance: this.state.data?.instance || "",
      peerInstance: this.machine(target)?.instance,
    }
  }
  async request(path: string, options: RequestInit = {}, timeout = 15000) {
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), timeout)
    try {
      const response = await this.fetcher(this.url(path), {
        ...options,
        signal: controller.signal,
      })
      if (response.status === 304)
        return { unchanged: true, value: null, tag: this.tag }
      const text = await response.text()
      if (!response.ok)
        throw new RequestError(text || "Request failed", response.status)
      return {
        unchanged: false,
        value: JSON.parse(text),
        tag: response.headers.get("ETag"),
      }
    } finally {
      clearTimeout(timer)
    }
  }
  async post(path: string, body: unknown, meta = this.meta()) {
    if (!meta.instance)
      throw new RequestError(
        "Waiting for Workbench to connect. Your draft is saved."
      )
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      "X-Command-Id": meta.id,
      "X-Command-Time": String(meta.issued),
      "X-Workbench-Instance": meta.instance,
    }
    if (meta.prompt) headers["X-Prompt-Id"] = meta.prompt
    const target = (body as { agent?: string })?.agent
    if (this.machine(target) && !meta.peerInstance)
      throw new RequestError(
        "Review this remote message before sending it again.",
        409
      )
    if (meta.peerInstance) headers["X-Peer-Instance"] = meta.peerInstance
    return (
      await this.request(path, {
        method: "POST",
        headers,
        body: JSON.stringify(body),
      })
    ).value
  }
  selectAgent = (id: string | null) => {
    const a = this.state.data?.agents.find((a) => a.id === id)
    if (a) {
      this.lastAgent[a.project_id] = a.id
      this.save("project-agents", this.lastAgent)
      this.save("project", a.project_id)
    }
    this.save("agent", id || "")
    this.tag = null
    this.emit({
      current: id,
      project: a?.project_id || this.state.project,
      thread: [],
      have: 0,
      epoch: "",
    })
    void this.refresh()
  }
  selectProject = (id: string) => {
    const agents =
      this.state.data?.agents.filter((a) => a.project_id === id) || []
    const a =
      agents.find((a) => a.id === this.lastAgent[id]) ||
      agents.find((a) => a.status === "blocked") ||
      agents[0]
    this.save("project", id)
    this.state.project = id
    this.selectAgent(a?.id || null)
  }
  async refresh() {
    if (this.refreshing) return
    this.refreshing = true
    const owner = this.state.current
    try {
      const result = await this.request(
        `/api/state?agent=${encodeURIComponent(owner || "")}&have=${this.state.have}&epoch=${encodeURIComponent(this.state.epoch)}`,
        {
          cache: "no-store",
          headers: this.tag ? { "If-None-Match": this.tag } : {},
        },
        20000
      )
      if (owner !== this.state.current) return
      if (result.unchanged) {
        this.emit({ connection: "online" })
        return
      }
      this.tag = result.tag
      const data = result.value as Snapshot
      this.clockOffset = Number.isFinite(data.at)
        ? data.at - Math.floor(Date.now() / 1000)
        : 0
      this.state.data = data
      let project = data.projects.find((p) => p.id === this.state.project)
      const a = data.agents.find((a) => a.id === owner)
      if (a && this.lastAgent[a.project_id] !== a.id) {
        this.lastAgent[a.project_id] = a.id
        this.save("project-agents", this.lastAgent)
      }
      if (!project && a)
        project = data.projects.find((p) => p.id === a.project_id)
      if (!project)
        project =
          data.projects.find((p) =>
            data.agents.some(
              (a) => a.project_id === p.id && a.status === "blocked"
            )
          ) || data.projects[0]
      if (!a || a.project_id !== project?.id) {
        this.state.project = project?.id || null
        const candidates = data.agents.filter(
          (a) => a.project_id === project?.id
        )
        const next =
          candidates.find((a) => a.id === this.lastAgent[project!.id]) ||
          candidates.find((a) => a.status === "blocked") ||
          candidates[0]
        this.emit({
          connection: "online",
          current: null,
          thread: [],
          have: 0,
          epoch: "",
        })
        if (next) this.selectAgent(next.id)
        return
      }
      const reset =
        a.msg_reset ||
        (a.msg_epoch && this.state.epoch && a.msg_epoch !== this.state.epoch)
      const thread = (
        reset ? a.messages : [...this.state.thread, ...a.messages]
      ).slice(-200)
      const said = thread.filter((m) => m.role === "you")
      let outbox = this.state.outbox.filter(
        (item) =>
          !(
            item.agent === owner &&
            item.status === "accepted" &&
            (item.kind === "todo" ||
              said.some(
                (m) =>
                  m.text === item.text || m.text.startsWith(item.text + "\n")
              ))
          )
      )
      const old = new Set(
        outbox
          .filter((i) => i.status === "accepted")
          .slice(0, -50)
          .map((i) => i.id)
      )
      outbox = outbox.filter((i) => !old.has(i.id))
      this.emit({
        data,
        project: project.id,
        connection: "online",
        thread,
        outbox,
        have: a.msg_total ?? this.state.have,
        epoch: a.msg_epoch || this.state.epoch,
      })
      this.persistOutbox()
    } catch (error) {
      this.emit({
        connection:
          error instanceof RequestError && error.status === 401
            ? "unauthorized"
            : "offline",
      })
    } finally {
      this.refreshing = false
      if (owner !== this.state.current)
        queueMicrotask(() => void this.refresh())
    }
  }
  submit = async (kind: "reply" | "todo" = "reply") => {
    const owner = this.state.current
    if (!owner) return
    if (!this.requireAvailable(owner)) return
    if (!this.state.data?.instance) {
      this.notice("Waiting for Workbench to connect. Your draft is saved.")
      return
    }
    if (this.state.uploading[owner]) {
      this.notice("Wait for your attachments to finish uploading.")
      return
    }
    const draft = this.draft(owner)
    const text = [draft.text.trim(), ...draft.attached.map((a) => a.path)]
      .filter(Boolean)
      .join("\n")
    if (!text) return
    let item = this.state.outbox.find(
      (i) =>
        i.agent === owner &&
        i.kind === kind &&
        i.text === text &&
        i.status !== "accepted"
    )
    if (!item) {
      item = {
        ...this.meta(owner),
        agent: owner,
        text,
        kind,
        status: "pending",
        draftRevision: draft.revision,
        typed: draft.text,
        attached: [...draft.attached],
      }
      this.state.outbox = [...this.state.outbox, item]
    }
    return this.deliver(item.id)
  }
  async deliver(id: string) {
    const item = this.state.outbox.find((i) => i.id === id)
    if (!item || item.inflight || item.status === "accepted") return
    if (!this.requireAvailable(item.agent)) return
    item.inflight = true
    item.status = "pending"
    item.error = ""
    this.persistOutbox()
    this.emit()
    try {
      await this.post(
        "/api/" + item.kind,
        { agent: item.agent, text: item.text },
        item
      )
      item.status = "accepted"
      const draft = this.draft(item.agent)
      if (draft.revision === item.draftRevision)
        this.writeDraft(item.agent, {
          ...blankDraft(),
          revision: draft.revision + 1,
        })
      for (const file of item.attached) {
        if (file.preview) URL.revokeObjectURL(file.preview)
      }
      this.notice(
        item.kind === "todo" ? "Added to the queue." : "Accepted by Workbench."
      )
      return true
    } catch (error) {
      item.status =
        error instanceof RequestError && error.status > 0 && error.status < 500
          ? "failed"
          : "unknown"
      item.error =
        error instanceof Error ? error.message : "Delivery is unconfirmed."
      this.notice(item.error)
      return false
    } finally {
      item.inflight = false
      this.persistOutbox()
      this.emit()
    }
  }
  editDelivery(id: string) {
    const item = this.state.outbox.find((i) => i.id === id)
    if (!item || item.inflight || item.status !== "failed") return
    const draft = this.draft(item.agent)
    if (draft.text && draft.text !== item.typed) {
      this.notice(
        "Save or send the current draft before restoring this message."
      )
      return
    }
    this.writeDraft(item.agent, {
      text: item.typed,
      attached: item.attached,
      revision: draft.revision + 1,
    })
    this.state.outbox = this.state.outbox.filter((i) => i.id !== id)
    this.persistOutbox()
    this.selectAgent(item.agent)
  }
  async action(
    path: string,
    body: { agent: string; text: string },
    prompt?: string
  ) {
    if (!this.requireAvailable(body.agent)) return
    const key = path + JSON.stringify(body) + (prompt || "")
    let item = this.actions.get(key)
    if (item?.pending) return
    if (!item) {
      item = { ...this.meta(body.agent), prompt, pending: false }
      this.actions.set(key, item)
    }
    item.pending = true
    this.emit({
      pendingActions: [...this.actions]
        .filter(([, i]) => i.pending)
        .map(([k]) => k),
    })
    try {
      await this.post(path, body, item)
      this.actions.delete(key)
      this.notice("Accepted by Workbench.")
      await this.refresh()
      return true
    } catch (error) {
      if (
        error instanceof RequestError &&
        error.status > 0 &&
        error.status < 500
      )
        this.actions.delete(key)
      this.notice(
        error instanceof Error
          ? error.message
          : "Connection interrupted. Try again."
      )
      return false
    } finally {
      item.pending = false
      this.emit({
        pendingActions: [...this.actions]
          .filter(([, i]) => i.pending)
          .map(([k]) => k),
      })
    }
  }
  answer(key: string, prompt: string, owner: string) {
    if (owner !== this.state.current || this.agent?.prompt?.id !== prompt) {
      this.notice("That question has changed. Review it again.")
      return
    }
    return this.action("/api/answer", { agent: owner, text: key }, prompt)
  }
  async upload(files: File[], owner = this.state.current) {
    if (!owner) return
    if (!this.requireAvailable(owner)) return
    this.emit({
      uploading: {
        ...this.state.uploading,
        [owner]: (this.state.uploading[owner] || 0) + files.length,
      },
    })
    for (const file of files) {
      try {
        if (file.size > 25 * 1024 * 1024)
          throw Error("Files must be smaller than 25 MB.")
        const result = await this.request(
          `/api/upload?agent=${encodeURIComponent(owner)}&name=${encodeURIComponent(file.name)}`,
          { method: "POST", body: file },
          75000
        )
        const d = this.draft(owner)
        this.writeDraft(owner, {
          ...d,
          attached: [
            ...d.attached,
            {
              path: result.value.path,
              name: file.name,
              preview: file.type.startsWith("image/")
                ? URL.createObjectURL(file)
                : undefined,
            },
          ],
          revision: d.revision + 1,
        })
      } catch (error) {
        this.notice(
          `Could not upload ${file.name}: ${error instanceof Error ? error.message : "Try again."}`
        )
      } finally {
        this.emit({
          uploading: {
            ...this.state.uploading,
            [owner]: this.state.uploading[owner] - 1,
          },
        })
      }
    }
  }
  removeAttachment(index: number) {
    const owner = this.state.current
    if (!owner) return
    const d = this.draft(owner)
    if (d.attached[index]?.preview)
      URL.revokeObjectURL(d.attached[index].preview!)
    this.writeDraft(owner, {
      ...d,
      attached: d.attached.filter((_, i) => i !== index),
      revision: d.revision + 1,
    })
  }
}
