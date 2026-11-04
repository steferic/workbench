"use client"
/* eslint-disable @next/next/no-img-element -- Authenticated media is served at its original size by Rust, with no Next image server. */
import { useEffect, useLayoutEffect, useRef, useState } from "react"
import { AnimatePresence } from "motion/react"
import Markdown from "react-markdown"
import remarkGfm from "remark-gfm"
import {
  IconArrowDown,
  IconTerminal2,
  IconDownload,
  IconPhoto,
} from "@tabler/icons-react"
import { ChatMessage } from "@/components/ui/chat-message"
import { ThinkingIndicator } from "@/components/ui/thinking-indicator"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog"
import type { Agent, ClientState, Media } from "@/lib/workbench/types"
import type { WorkbenchClient } from "@/lib/workbench/client"
import { outcome } from "./sounds"

const time = (at?: string) =>
  at
    ? new Date(at).toLocaleTimeString([], {
        hour: "numeric",
        minute: "2-digit",
      })
    : undefined
export function Conversation({
  client,
  state,
  agent,
}: {
  client: WorkbenchClient
  state: ClientState
  agent: Agent
}) {
  const scroll = useRef<HTMLDivElement>(null)
  const follow = useRef(true)
  const [away, setAway] = useState(false)
  const [media, setMedia] = useState<Media | null>(null)
  const mediaRef = useRef<Media | null>(null)
  const items = state.data?.media.filter((m) => m.agent === agent.id) || []
  const deliveries = state.outbox.filter((i) => i.agent === agent.id)
  useLayoutEffect(() => {
    if (follow.current && scroll.current)
      scroll.current.scrollTop = scroll.current.scrollHeight
  }, [state.thread, agent.tail, state.outbox, items.length])
  useEffect(() => {
    const observer = new ResizeObserver(() => {
      if (follow.current && scroll.current)
        scroll.current.scrollTop = scroll.current.scrollHeight
    })
    if (scroll.current?.firstElementChild)
      observer.observe(scroll.current.firstElementChild)
    return () => observer.disconnect()
  }, [])
  useEffect(() => {
    const close = () => {
      mediaRef.current = null
      setMedia(null)
    }
    addEventListener("popstate", close)
    return () => {
      removeEventListener("popstate", close)
      if (mediaRef.current && history.state?.workbenchMedia) history.back()
    }
  }, [])
  function openMedia(item: Media) {
    history.pushState({ workbenchMedia: item.id }, "")
    mediaRef.current = item
    setMedia(item)
  }
  function closeMedia() {
    mediaRef.current = null
    setMedia(null)
    if (history.state?.workbenchMedia) history.back()
  }
  const mediaCard = (item: Media) => (
    <Button
      variant="ghost"
      key={`media-${item.id}`}
      className="media-card"
      onClick={() => openMedia(item)}
      data-cuelume-open
    >
      {item.kind === "image" || item.poster ? (
        <img
          src={client.url(
            `/media/${encodeURIComponent(item.id)}/${item.kind === "video" ? "poster" : "file"}`
          )}
          alt={item.name}
          loading="lazy"
        />
      ) : (
        <IconPhoto />
      )}
      <span>
        {item.kind === "video" ? "Play" : "View"} {item.name}
      </span>
    </Button>
  )
  const remaining = [...items].sort((a, b) =>
    (a.at || "").localeCompare(b.at || "")
  )
  return (
    <div className="transcript-wrap">
      <div
        ref={scroll}
        className="transcript"
        aria-label="Conversation"
        tabIndex={0}
        onScroll={(e) => {
          const el = e.currentTarget
          follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80
          setAway(!follow.current)
        }}
      >
        <div className="transcript-inner">
          {state.have > state.thread.length && (
            <p className="history-note">
              Showing the latest {state.thread.length} entries
            </p>
          )}
          <AnimatePresence initial={false}>
            {state.thread.flatMap((m, index) => {
              const before = []
              while (
                m.at &&
                remaining.length &&
                remaining[0].at &&
                remaining[0].at! <= m.at
              )
                before.push(mediaCard(remaining.shift()!))
              const key = `${state.epoch}-${state.have - state.thread.length + index}`
              if (m.role === "tool")
                before.push(
                  <div className="tool-entry" key={key}>
                    <IconTerminal2 size={14} aria-hidden />
                    <span>{m.text}</span>
                  </div>
                )
              else
                before.push(
                  <ChatMessage
                    key={key}
                    from={m.role === "you" ? "user" : "assistant"}
                    time={time(m.at)}
                    className={`chat-entry ${m.role === "you" ? "chat-user" : "chat-agent"}`}
                  >
                    <div className="typeset message-prose">
                      <Markdown
                        remarkPlugins={[remarkGfm]}
                        components={{
                          a: ({ children, href }) => (
                            <a
                              href={href}
                              target="_blank"
                              rel="noopener noreferrer"
                            >
                              {children}
                            </a>
                          ),
                          // Untrusted journal images must not make background requests to remote hosts.
                          img: ({ alt }) => <span>{alt || "Image"}</span>,
                        }}
                      >
                        {m.text}
                      </Markdown>
                    </div>
                  </ChatMessage>
                )
              return before
            })}
            {remaining.map(mediaCard)}
          </AnimatePresence>
          {!state.thread.length && agent.tail.length > 0 && (
            <pre className="terminal-output" aria-label="Terminal output">
              {agent.tail.join("\n")}
            </pre>
          )}
          {!state.thread.length && !agent.tail.length && !items.length && (
            <div className="conversation-empty">
              <h2>
                {agent.status === "stopped"
                  ? "Pick up where you left off"
                  : "Ready when you are"}
              </h2>
              <p>
                {agent.status === "stopped"
                  ? "Send a message to wake this agent."
                  : "Send a message to start this conversation."}
              </p>
            </div>
          )}
          {deliveries.map((item) => (
            <div key={item.id} className="delivery-entry">
              <p className="delivery-text">{item.text}</p>
              <span className="muted">
                {
                  {
                    pending: "Sending…",
                    accepted: "Accepted by Workbench",
                    failed: "Not accepted",
                    unknown: "Delivery unconfirmed",
                  }[item.status]
                }
              </span>
              {item.error && <p>{item.error}</p>}
              {item.status === "unknown" && (
                <Button
                  variant="secondary"
                  onClick={() => void outcome(client.deliver(item.id))}
                >
                  Retry safely
                </Button>
              )}
              {item.status === "failed" && (
                <Button
                  variant="secondary"
                  onClick={() => client.editDelivery(item.id)}
                >
                  Edit draft
                </Button>
              )}
            </div>
          ))}
          {agent.status === "working" && (
            <div className="working-indicator">
              <ThinkingIndicator />
            </div>
          )}
        </div>
      </div>
      {away && (
        <Button
          variant="secondary"
          leadingIcon={IconArrowDown}
          className="latest-button"
          onClick={() => {
            follow.current = true
            setAway(false)
            if (scroll.current)
              scroll.current.scrollTop = scroll.current.scrollHeight
          }}
        >
          Latest
        </Button>
      )}
      <Dialog
        open={!!media}
        onOpenChange={(open) => {
          if (!open) closeMedia()
        }}
      >
        <DialogContent size="xl" className="workbench-dialog media-dialog">
          <DialogTitle>{media?.name}</DialogTitle>
          {media && (
            <>
              {media.kind === "video" ? (
                <video
                  src={client.url(
                    `/media/${encodeURIComponent(media.id)}/file`
                  )}
                  controls
                  playsInline
                  autoPlay
                />
              ) : (
                <img
                  src={client.url(
                    `/media/${encodeURIComponent(media.id)}/file`
                  )}
                  alt={media.name}
                />
              )}
              <Button
                variant="secondary"
                leadingIcon={IconDownload}
                render={
                  <a
                    href={client.url(
                      `/media/${encodeURIComponent(media.id)}/file`
                    )}
                    download={media.name}
                  >
                    Download
                  </a>
                }
              />
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
