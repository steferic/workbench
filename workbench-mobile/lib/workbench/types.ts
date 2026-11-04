export type Message = {
  role: "you" | "agent" | "tool"
  text: string
  at?: string
}
export type Prompt = {
  id: string
  lines: string[]
  options: { key: string; label: string }[]
}
export type Agent = {
  id: string
  project_id: string
  project: string
  provider: string
  model?: string
  alias?: string
  status: "idle" | "working" | "blocked" | "stopped"
  manager?: boolean
  reason?: string
  running?: string
  holding?: string
  queued: string[]
  steps: { text: string; state: string }[]
  prompt?: Prompt
  messages: Message[]
  tail: string[]
  msg_total: number
  msg_reset: boolean
  msg_epoch: string
}
export type Project = {
  id: string
  name: string
  path: string
  global?: boolean
  jobs: {
    id: string
    title: string
    running: boolean
    due?: boolean
    last_status?: string
  }[]
  objectives: { id: string; text: string; state: string; done_when?: string }[]
  proposals: {
    id: string
    instruction: string
    state: string
    phase?: string
    why?: string
  }[]
}
export type Media = {
  id: string
  agent: string
  name: string
  kind: string
  poster?: boolean
  at?: string
}
export type Decision = {
  id: string
  project: string
  agent?: string
  kind: string
  title: string
  detail?: string
}
export type Snapshot = {
  instance: string
  at: number
  projects: Project[]
  agents: Agent[]
  media: Media[]
  desk: Decision[]
  machines?: Machine[]
}
export type Machine = {
  name: string
  status: "online" | "offline" | "connecting"
  instance: string
}
export type Attachment = { path: string; name: string; preview?: string }
export type Draft = { text: string; attached: Attachment[]; revision: number }
export type CommandMeta = {
  id: string
  issued: number
  instance: string
  prompt?: string
  peerInstance?: string
}
export type Delivery = CommandMeta & {
  agent: string
  text: string
  kind: "reply" | "todo"
  status: "pending" | "accepted" | "failed" | "unknown"
  typed: string
  attached: Attachment[]
  draftRevision: number
  inflight?: boolean
  error?: string
}
export type ClientState = {
  data: Snapshot | null
  current: string | null
  project: string | null
  thread: Message[]
  have: number
  epoch: string
  drafts: Record<string, Draft>
  outbox: Delivery[]
  connection: "connecting" | "online" | "offline" | "unauthorized"
  notice: string
  pendingActions: string[]
  uploading: Record<string, number>
}
