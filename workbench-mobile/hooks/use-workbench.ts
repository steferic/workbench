"use client"
import { useEffect, useState, useSyncExternalStore } from "react"
import { WorkbenchClient } from "@/lib/workbench/client"

export function useWorkbench() {
  const [client, setClient] = useState<WorkbenchClient | null>(null)
  useEffect(() => {
    let storage: Storage | undefined
    try {
      storage = localStorage
    } catch {}
    const token = new URLSearchParams(location.search).get("t") || ""
    const next = new WorkbenchClient(token, storage)
    // Construct the browser transport after hydration; the server has no token or storage.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setClient(next)
    void next.refresh()
    const timer = setInterval(() => {
      if (!document.hidden) void next.refresh()
    }, 1000)
    const refresh = () => void next.refresh()
    addEventListener("online", refresh)
    document.addEventListener("visibilitychange", refresh)
    const manifest = document.createElement("link")
    manifest.rel = "manifest"
    manifest.href = next.url("/manifest.webmanifest")
    document.head.append(manifest)
    return () => {
      clearInterval(timer)
      removeEventListener("online", refresh)
      document.removeEventListener("visibilitychange", refresh)
      manifest.remove()
    }
  }, [])
  return client
}
export function useClientState(client: WorkbenchClient) {
  return useSyncExternalStore(
    client.subscribe,
    client.getSnapshot,
    client.getSnapshot
  )
}
