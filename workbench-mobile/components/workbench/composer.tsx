"use client"
/* eslint-disable @next/next/no-img-element -- Attachment previews use local blob URLs. */
import { useEffect, useRef, useState } from "react"
import { play } from "cuelume"
import {
  IconPaperclip,
  IconMicrophone,
  IconPlayerStop,
  IconListCheck,
  IconArrowUp,
  IconX,
} from "@tabler/icons-react"
import { Button } from "@/components/ui/button"
import { InputMessage } from "@/components/ui/input-message"
import type { Agent, ClientState } from "@/lib/workbench/types"
import { WorkbenchClient, agentName } from "@/lib/workbench/client"
import { outcome } from "./sounds"

type Speech = {
  continuous: boolean
  interimResults: boolean
  lang: string
  start(): void
  stop(): void
  onresult:
    | ((e: { results: ArrayLike<ArrayLike<{ transcript: string }>> }) => void)
    | null
  onerror: ((e: { error: string }) => void) | null
  onend: (() => void) | null
}

export function Composer({
  client,
  state,
  agent,
}: {
  client: WorkbenchClient
  state: ClientState
  agent: Agent
}) {
  const files = useRef<HTMLInputElement>(null)
  const speech = useRef<Speech | null>(null)
  const speechActive = useRef(false)
  const [listening, setListening] = useState(false)
  const draft = client.draft()
  const uploading = state.uploading[agent.id] || 0
  const sending = state.outbox.some((i) => i.agent === agent.id && i.inflight)
  const unavailable = !client.available(agent.id)
  const locked = sending || !!uploading || unavailable
  const prompt = agent.prompt
  const answering =
    !!prompt && state.pendingActions.some((key) => key.endsWith(prompt.id))
  useEffect(
    () => () => {
      speechActive.current = false
      if (speech.current) {
        speech.current.onresult = null
        speech.current.stop()
      }
    },
    []
  )

  function dictate() {
    if (listening) {
      speech.current?.stop()
      return
    }
    const browser = window as unknown as {
      SpeechRecognition?: new () => Speech
      webkitSpeechRecognition?: new () => Speech
    }
    const Recognition =
      browser.SpeechRecognition || browser.webkitSpeechRecognition
    if (!Recognition) {
      client.notice(
        "Dictation isn’t available in this browser. You can use the microphone on your keyboard."
      )
      return
    }
    const recognition = new Recognition()
    speech.current = recognition
    recognition.continuous = true
    recognition.interimResults = true
    recognition.lang = navigator.language
    const owner = agent.id
    const before = draft.text ? draft.text + " " : ""
    recognition.onresult = (e) => {
      if (!speechActive.current || client.state.current !== owner) return
      client.setText(
        before +
          Array.from(e.results)
            .map((result) => result[0].transcript)
            .join(""),
        owner
      )
    }
    recognition.onerror = (e) => {
      speechActive.current = false
      setListening(false)
      client.notice(
        e.error === "not-allowed"
          ? "Microphone permission was denied. You can use the microphone on your keyboard."
          : `Dictation stopped: ${e.error}`
      )
    }
    recognition.onend = () => {
      speechActive.current = false
      setListening(false)
    }
    try {
      speechActive.current = true
      recognition.start()
      setListening(true)
    } catch {
      speechActive.current = false
      client.notice(
        "Could not start dictation. Try your keyboard’s microphone."
      )
    }
  }
  function submit(kind: "reply" | "todo" = "reply") {
    if (locked) return
    speechActive.current = false
    speech.current?.stop()
    play("tap", { emphasis: "subtle" })
    void outcome(client.submit(kind))
  }
  return (
    <footer className="composer-area">
      {prompt?.id && (
        <section
          className="approval"
          aria-label="Question waiting for your answer"
        >
          <div className="approval-heading">
            <span className="status-dot blocked" />
            <h2>Needs your answer</h2>
          </div>
          <p>{prompt.lines.join("\n")}</p>
          <div className="approval-options">
            {prompt.options.map((option, i) => (
              <Button
                key={option.key}
                variant={i === 0 ? "primary" : "secondary"}
                disabled={answering || unavailable}
                onClick={() =>
                  void outcome(client.answer(option.key, prompt.id, agent.id))
                }
                data-cuelume-tap
              >
                {option.label}
              </Button>
            ))}
          </div>
        </section>
      )}
      {!!agent.queued.length && (
        <p className="queue-note">
          <IconListCheck size={15} />
          {agent.queued.length}{" "}
          {agent.queued.length === 1 ? "message" : "messages"} queued
          {agent.holding ? ` · ${agent.holding}` : ""}
        </p>
      )}
      {!!draft.attached.length && (
        <div className="attachments" aria-label="Attachments">
          {draft.attached.map((attachment, i) => (
            <div className="attachment" key={attachment.path}>
              {attachment.preview && <img src={attachment.preview} alt="" />}
              <span>{attachment.name}</span>
              <Button
                variant="ghost"
                size="icon"
                aria-label={`Remove ${attachment.name}`}
                onClick={() => client.removeAttachment(i)}
              >
                <IconX />
              </Button>
            </div>
          ))}
        </div>
      )}
      {!!uploading && (
        <p className="queue-note" role="status">
          Uploading {uploading} {uploading === 1 ? "file" : "files"}…
        </p>
      )}
      <input
        ref={files}
        type="file"
        multiple
        hidden
        aria-label="Attach files"
        onChange={(e) => {
          const selected = Array.from(e.target.files || [])
          e.target.value = ""
          void client.upload(selected, agent.id)
        }}
      />
      <InputMessage
        value={draft.text}
        onValueChange={(text) => client.setText(text)}
        onSend={() => submit()}
        placeholder={`Message ${agentName(agent)}…`}
        minRows={1}
        maxRows={4}
        sendLabel="Send message"
        className={`workbench-composer ${(!draft.text.trim() && draft.attached.length) || unavailable ? "custom-send" : ""}`}
        onKeyDownCapture={(e) => {
          if (
            e.key === "Enter" &&
            !e.shiftKey &&
            !e.metaKey &&
            !e.ctrlKey &&
            window.matchMedia("(pointer: coarse)").matches
          )
            e.stopPropagation()
        }}
        textareaProps={{
          "aria-label": `Message ${agentName(agent)}`,
          autoCapitalize: "sentences",
          spellCheck: true,
        }}
        leftSlot={
          <>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Attach files"
              disabled={unavailable}
              onClick={() => files.current?.click()}
              data-cuelume-open
            >
              <IconPaperclip />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label={listening ? "Stop dictation" : "Dictate message"}
              aria-pressed={listening}
              active={listening}
              onClick={dictate}
            >
              {listening ? <IconPlayerStop /> : <IconMicrophone />}
            </Button>
          </>
        }
        rightSlot={
          <>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Queue message"
              title="Send after the current turn finishes"
              disabled={
                locked || (!draft.text.trim() && !draft.attached.length)
              }
              onClick={() => submit("todo")}
            >
              <IconListCheck />
            </Button>
            {((!draft.text.trim() && !!draft.attached.length) ||
              unavailable) && (
              <Button
                size="icon"
                aria-label={
                  unavailable ? "Send when connected" : "Send attachments"
                }
                disabled={locked}
                onClick={() => submit()}
              >
                <IconArrowUp />
              </Button>
            )}
          </>
        }
      />
    </footer>
  )
}
