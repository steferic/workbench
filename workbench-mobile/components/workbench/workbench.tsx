"use client"
import { useEffect, useState } from "react"
import { useTheme } from "next-themes"
import {
  IconLayoutSidebar,
  IconFolder,
  IconSettings,
  IconChecks,
  IconPlus,
  IconX,
  IconDots,
  IconArrowRight,
  IconBell,
  IconTerminal2,
} from "@tabler/icons-react"
import {
  Sidebar,
  SidebarProvider,
  SidebarHeader,
  SidebarContent,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  useSidebar,
} from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogTrigger,
} from "@/components/ui/dialog"
import {
  Select,
  SelectTrigger,
  SelectContent,
  SelectItem,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Badge } from "@/components/ui/badge"
import { Banner, BannerTitle, BannerDescription } from "@/components/ui/banner"
import { Skeleton } from "@/components/ui/skeleton"
import { useWorkbench, useClientState } from "@/hooks/use-workbench"
import { WorkbenchClient, agentName } from "@/lib/workbench/client"
import { notifications } from "@/lib/workbench/notifications"
import { fontWeights } from "@/lib/font-weight"
import { typeClass } from "@/lib/type-scale"
import { useSounds, outcome } from "./sounds"
import { Conversation } from "./conversation"
import { Composer } from "./composer"
import type { Project, Decision } from "@/lib/workbench/types"

export function Workbench() {
  const client = useWorkbench()
  if (!client)
    return (
      <div className="startup" role="status">
        <IconTerminal2 />
        <span>Connecting to Workbench…</span>
        <Skeleton className="h-3 w-36" />
      </div>
    )
  return (
    <SidebarProvider
      persist={false}
      width="248px"
      widthMobile="min(320px, 88vw)"
      className="workbench-shell"
    >
      <Shell client={client} />
    </SidebarProvider>
  )
}

function Shell({ client }: { client: WorkbenchClient }) {
  const state = useClientState(client)
  const { toggleSidebar, setOpenMobile, openMobile, isMobile } = useSidebar()
  const [details, setDetails] = useState(false)
  const [desk, setDesk] = useState(false)
  const { data, current, connection } = state
  const project = data?.projects.find((p) => p.id === state.project)
  const machine = client.machine(project?.id || null)
  const a = data?.agents.find((a) => a.id === current)
  const siblings =
    data?.agents.filter((a) => a.project_id === project?.id) || []

  useEffect(() => {
    const fit = () => {
      const view = window.visualViewport
      if (view && Math.abs(view.scale - 1) < 0.01) {
        document.documentElement.style.setProperty(
          "--app-height",
          `${view.height}px`
        )
        document.documentElement.style.setProperty(
          "--app-top",
          `${view.offsetTop}px`
        )
      } else {
        document.documentElement.style.removeProperty("--app-height")
        document.documentElement.style.removeProperty("--app-top")
      }
    }
    fit()
    window.visualViewport?.addEventListener("resize", fit)
    window.visualViewport?.addEventListener("scroll", fit)
    return () => {
      window.visualViewport?.removeEventListener("resize", fit)
      window.visualViewport?.removeEventListener("scroll", fit)
    }
  }, [])

  return (
    <>
      <Sidebar className="project-sidebar" aria-label="Projects">
        <SidebarHeader className="project-header">
          <div className="brand">
            <IconTerminal2 aria-hidden />
            <span style={{ fontVariationSettings: fontWeights.semibold }}>
              Workbench
            </span>
            {isMobile && (
              <Button
                variant="ghost"
                size="icon"
                aria-label="Close projects"
                onClick={() => setOpenMobile(false)}
                data-cuelume-close
              >
                <IconX />
              </Button>
            )}
          </div>
          <h2 className={typeClass("title")}>Projects</h2>
        </SidebarHeader>
        <SidebarContent>
          <nav aria-label="Projects" className="project-list">
            <SidebarMenu>
              {data?.projects.map((p) => (
                <SidebarMenuItem key={p.id}>
                  <SidebarMenuButton
                    icon={IconFolder}
                    isActive={p.id === state.project}
                    aria-current={p.id === state.project ? "page" : undefined}
                    onClick={() => {
                      client.selectProject(p.id)
                      setOpenMobile(false)
                    }}
                    data-cuelume-navigate
                    className="project-row"
                  >
                    {p.name}
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
            {data && !data.projects.length && (
              <p className="muted empty-projects">
                Add a project in Workbench on your Mac to see it here.
              </p>
            )}
            {!data && (
              <div className="loading-projects">
                <Skeleton className="h-10 w-full" />
                <Skeleton className="h-10 w-full" />
              </div>
            )}
          </nav>
        </SidebarContent>
      </Sidebar>
      <div className="workspace">
        <header className="topbar">
          <Button
            variant="ghost"
            size="icon"
            aria-label="Projects"
            aria-expanded={isMobile ? openMobile : undefined}
            onClick={toggleSidebar}
            data-cuelume-open
          >
            <IconLayoutSidebar />
          </Button>
          <div className="project-title">
            <span className="product-name">Workbench</span>
            <h1 className={typeClass("title")}>
              {project?.name || "Your projects"}
            </h1>
          </div>
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Decisions${data?.desk.length ? `, ${data.desk.length} waiting` : ""}`}
            onClick={() => setDesk(true)}
            className="decision-trigger"
            data-cuelume-open
          >
            <IconChecks />
            {!!data?.desk.length && <span className="attention-dot" />}
          </Button>
          <Settings client={client} />
        </header>
        <main className="conversation-sheet">
          <div className="conversation-toolbar">
            {a ? (
              <Select value={a.id} onValueChange={client.selectAgent}>
                <SelectTrigger
                  aria-label="Agent in this project"
                  className="agent-select"
                />
                <SelectContent>
                  {siblings.map((s, i) => (
                    <SelectItem index={i} value={s.id} key={s.id}>
                      {agentName(s) +
                        (siblings.filter((x) => agentName(x) === agentName(s))
                          .length > 1
                          ? ` · ${s.id}`
                          : "")}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : (
              <span className="muted">
                {data ? "No conversations yet" : "Connecting…"}
              </span>
            )}
            {a && (
              <span className="agent-status">
                <span className={`status-dot ${a.status}`} />
                {machine && machine.status !== "online"
                  ? "Unavailable"
                  : a.status === "blocked"
                    ? "Needs you"
                    : a.status === "working"
                      ? "Working"
                      : a.status === "stopped"
                        ? "Stopped"
                        : "Ready"}
              </span>
            )}
            <Button
              variant="ghost"
              size="icon"
              aria-label="Project actions"
              onClick={() => setDetails(true)}
              disabled={!project}
              className="project-actions"
              data-cuelume-open
            >
              <IconDots />
            </Button>
          </div>
          {machine &&
            machine.status !== "online" &&
            connection === "online" && (
              <Banner
                status="warning"
                variant="fixed"
                className="remote-notice"
                role="status"
              >
                <BannerTitle>
                  {machine.status === "connecting"
                    ? `Connecting to ${machine.name}…`
                    : `${machine.name} is offline`}
                </BannerTitle>
                <BannerDescription>
                  Showing its last update. Your draft is saved while it
                  reconnects.
                </BannerDescription>
              </Banner>
            )}
          {connection !== "online" && (
            <div className="connection-notice" role="status">
              {connection === "unauthorized"
                ? "This link has expired. Open the current phone link from Workbench on your Mac."
                : connection === "offline"
                  ? "Reconnecting to your Mac… Your draft is saved."
                  : "Connecting to your Mac…"}
              {connection === "offline" && (
                <Button variant="ghost" onClick={() => void client.refresh()}>
                  Retry
                </Button>
              )}
            </div>
          )}
          {a ? (
            <Conversation
              key={`conversation-${a.id}`}
              client={client}
              state={state}
              agent={a}
            />
          ) : (
            <div className="empty-conversation">
              <IconTerminal2 size={28} stroke={1.4} />
              <h2 className={typeClass("display", "compact")}>
                {data
                  ? project
                    ? `Start in ${project.name}`
                    : "Your work, within reach"
                  : "Opening your workspace"}
              </h2>
              <p>
                {data
                  ? project
                    ? "Start an agent to work on this project. Your conversation will appear here."
                    : "Add a project in Workbench on your Mac to get started."
                  : "Your projects and conversations will appear here."}
              </p>
              {project && (
                <div className="button-row">
                  <Button
                    leadingIcon={IconPlus}
                    disabled={
                      !client.available(project.id) ||
                      !!state.pendingActions.length
                    }
                    onClick={() =>
                      void outcome(
                        client.action("/api/new-agent", {
                          agent: project.id,
                          text: "codex",
                        })
                      )
                    }
                    data-cuelume-tap
                  >
                    Start Codex
                  </Button>
                  <Button
                    variant="secondary"
                    disabled={
                      !client.available(project.id) ||
                      !!state.pendingActions.length
                    }
                    onClick={() =>
                      void outcome(
                        client.action("/api/new-agent", {
                          agent: project.id,
                          text: "claude",
                        })
                      )
                    }
                    data-cuelume-tap
                  >
                    Start Claude
                  </Button>
                </div>
              )}
            </div>
          )}
          {state.notice && (
            <div className="feedback" role="status">
              <span>{state.notice}</span>
              <Button
                variant="ghost"
                size="icon"
                aria-label="Dismiss message"
                onClick={() => client.notice("")}
              >
                <IconX />
              </Button>
            </div>
          )}
          {a && (
            <Composer
              key={`composer-${a.id}`}
              client={client}
              state={state}
              agent={a}
            />
          )}
        </main>
      </div>
      <Dialog open={details} onOpenChange={setDetails}>
        <DialogContent size="lg" className="workbench-dialog">
          <DialogTitle>{project?.name}</DialogTitle>
          <DialogDescription>Project activity and actions</DialogDescription>
          {project && <ProjectDetails client={client} project={project} />}
        </DialogContent>
      </Dialog>
      <Dialog open={desk} onOpenChange={setDesk}>
        <DialogContent size="lg" className="workbench-dialog">
          <DialogTitle>Waiting on you</DialogTitle>
          <DialogDescription>Decisions across your projects</DialogDescription>
          {!data?.desk.length && (
            <p className="panel-empty">
              You’re all caught up. Decisions that need you will appear here.
            </p>
          )}
          {data?.desk.map((row) => (
            <DecisionRow
              key={row.kind + row.id}
              row={row}
              client={client}
              onOpen={() => setDesk(false)}
            />
          ))}
        </DialogContent>
      </Dialog>
    </>
  )
}

function Settings({ client }: { client: WorkbenchClient }) {
  const { theme, setTheme } = useTheme()
  const sound = useSounds()
  const [push, setPush] = useState(false)
  const [registering, setRegistering] = useState(false)
  const [error, setError] = useState("")
  const state = useClientState(client)
  useEffect(() => {
    if (!state.data?.instance) return
    try {
      if (localStorage.getItem("push") !== "on") return
    } catch {
      return
    }
    void notifications(client, true)
      .then(setPush)
      .catch(() =>
        setError(
          "Notification registration could not be confirmed. Try enabling it again."
        )
      )
  }, [client, state.data?.instance])
  return (
    <Dialog>
      <DialogTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            aria-label="Settings"
            data-cuelume-open
          >
            <IconSettings />
          </Button>
        }
      />
      <DialogContent className="workbench-dialog">
        <DialogTitle>Settings</DialogTitle>
        <DialogDescription>Make Workbench feel at home.</DialogDescription>
        <section className="setting" aria-labelledby="computers-heading">
          <h3 id="computers-heading">Computers</h3>
          <p className="muted">
            Projects from connected computers appear together in Projects.
          </p>
          <div className="computer-list">
            <div className="detail-row">
              <span>This Workbench</span>
              <Badge
                variant="dot"
                color={state.connection === "online" ? "accent" : "gray"}
              >
                {state.connection === "online" ? "Connected" : "Reconnecting"}
              </Badge>
            </div>
            {state.data?.machines?.map((computer) => (
              <div className="detail-row" key={computer.name}>
                <span>{computer.name}</span>
                <Badge
                  variant="dot"
                  color={computer.status === "online" ? "accent" : "gray"}
                >
                  {computer.status === "online"
                    ? "Connected"
                    : computer.status === "offline"
                      ? "Offline"
                      : "Connecting"}
                </Badge>
              </div>
            ))}
          </div>
          {!state.data?.machines?.length && (
            <p className="muted">
              Connect another computer from Workbench on your Mac.
            </p>
          )}
        </section>
        <div className="setting">
          <label id="theme-label">Appearance</label>
          <Select value={theme || "system"} onValueChange={setTheme}>
            <SelectTrigger aria-labelledby="theme-label" />
            <SelectContent>
              {["system", "light", "dark"].map((t, i) => (
                <SelectItem value={t} index={i} key={t}>
                  {t[0].toUpperCase() + t.slice(1)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="setting">
          <Switch
            className="sound-switch"
            label="Interaction sounds"
            checked={sound.enabled}
            onToggle={sound.toggle}
          />
          <p className="muted">
            Quiet cues when you switch projects, send a message, or finish an
            action.
          </p>
        </div>
        <div className="setting">
          <h3>Notifications</h3>
          <p className="muted">
            Know when an agent needs you, even when you’re away.
          </p>
          <Button
            leadingIcon={IconBell}
            variant="secondary"
            disabled={registering || push}
            onClick={async () => {
              setRegistering(true)
              setError("")
              try {
                setPush(await notifications(client))
              } catch (e) {
                setError(
                  e instanceof Error
                    ? e.message
                    : "Could not enable notifications."
                )
              } finally {
                setRegistering(false)
              }
            }}
            data-cuelume-tap
          >
            {push
              ? "Notifications enabled"
              : registering
                ? "Enabling…"
                : "Enable notifications"}
          </Button>
          {error && <p role="alert">{error}</p>}
        </div>
        <div className="setting">
          <h3>Usage</h3>
          <Button
            variant="secondary"
            render={<a href={client.url("/words")}>Word cloud</a>}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}

function ProjectDetails({
  client,
  project,
}: {
  client: WorkbenchClient
  project: Project
}) {
  const state = useClientState(client)
  const busy = !!state.pendingActions.length || !client.available(project.id)
  return (
    <div className="project-details">
      <section>
        <h3>New conversation</h3>
        <div className="button-row">
          {["codex", "claude"].map((provider) => (
            <Button
              key={provider}
              leadingIcon={IconPlus}
              variant="secondary"
              disabled={busy}
              onClick={() =>
                void outcome(
                  client.action("/api/new-agent", {
                    agent: project.id,
                    text: provider,
                  })
                )
              }
              data-cuelume-tap
            >
              {provider === "codex" ? "Codex" : "Claude"}
            </Button>
          ))}
        </div>
      </section>
      {!!project.jobs?.length && (
        <section>
          <h3>Jobs</h3>
          {project.jobs.map((job) => (
            <div className="detail-row" key={job.id}>
              <div>
                <p>{job.title}</p>
                <span className="muted">
                  {job.running ? "Running" : job.last_status || "Ready to run"}
                </span>
              </div>
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() =>
                  void outcome(
                    client.action("/api/job", {
                      agent: project.id,
                      text: job.id,
                    })
                  )
                }
              >
                {job.running ? "Run again" : "Run"}
              </Button>
            </div>
          ))}
        </section>
      )}
      {!!project.objectives?.length && (
        <section>
          <h3>Priorities</h3>
          {project.objectives.map((o) => (
            <div className="detail-row" key={o.id}>
              <p>{o.text}</p>
              <Badge>{o.state}</Badge>
            </div>
          ))}
        </section>
      )}
      {!!project.proposals?.filter((p) => p.state !== "declined").length && (
        <section>
          <h3>Activity</h3>
          {project.proposals
            .filter((p) => p.state !== "declined")
            .map((p) => (
              <div key={p.id} className="activity">
                <span className="muted">{p.phase || p.state}</span>
                <p>{p.instruction}</p>
                {p.why && <p className="muted">{p.why}</p>}
              </div>
            ))}
        </section>
      )}
    </div>
  )
}

function DecisionRow({
  row,
  client,
  onOpen,
}: {
  row: Decision
  client: WorkbenchClient
  onOpen: () => void
}) {
  const { pendingActions } = useClientState(client)
  const busy = !!pendingActions.length || !client.available(row.id)
  const decide = (approve: boolean) => {
    const path =
      row.kind === "check"
        ? "/api/check"
        : row.kind === "needs_user" && approve
          ? "/api/rearm"
          : "/api/proposal"
    const text =
      row.kind === "check"
        ? approve
          ? "approve"
          : "drop"
        : row.kind === "needs_user" && approve
          ? "rearm"
          : approve
            ? "approve"
            : "decline"
    void outcome(client.action(path, { agent: row.id, text }))
  }
  return (
    <section className="decision">
      <span className="muted">
        {row.project}
        {row.agent ? ` · ${row.agent}` : ""}
      </span>
      <h3>{row.title}</h3>
      {row.detail && <p className="decision-detail">{row.detail}</p>}
      {!client.available(row.id) && (
        <p className="muted">
          {client.machine(row.id)?.name} is unavailable. This decision will be
          ready when it reconnects.
        </p>
      )}
      {row.kind === "blocked" ? (
        <Button
          variant="secondary"
          trailingIcon={IconArrowRight}
          onClick={() => {
            client.selectAgent(row.id)
            onOpen()
          }}
        >
          Open conversation
        </Button>
      ) : (
        <div className="button-row">
          <Button disabled={busy} onClick={() => decide(true)}>
            {row.kind === "needs_user" ? "Review again" : "Approve"}
          </Button>
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => decide(false)}
          >
            Decline
          </Button>
        </div>
      )}
    </section>
  )
}
