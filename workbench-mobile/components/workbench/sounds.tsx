"use client"
import { createContext, useContext, useEffect, useState } from "react"
import { bind, play, setEnabled, setVolume, unlock } from "cuelume"

const SoundContext = createContext({ enabled: true, toggle: () => {} })
export function SoundProvider({ children }: { children: React.ReactNode }) {
  const [enabled, update] = useState(true)
  useEffect(() => {
    let on = true
    try {
      on = localStorage.getItem("workbench-sounds") !== "off"
    } catch {}
    // The persisted browser preference is intentionally read after hydration.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    update(on)
    setEnabled(on)
    setVolume(0.32)
    const unbind = bind()
    const haptic = (event: MouseEvent) => {
      const button =
        event.target instanceof Element ? event.target.closest("button") : null
      if (button && !button.disabled) navigator.vibrate?.(12)
    }
    document.addEventListener("click", haptic)
    return () => {
      unbind()
      document.removeEventListener("click", haptic)
    }
  }, [])
  function toggle() {
    const on = !enabled
    update(on)
    setEnabled(on)
    try {
      localStorage.setItem("workbench-sounds", on ? "on" : "off")
    } catch {}
    if (on) void unlock().then(() => play("toggle", { emphasis: "subtle" }))
  }
  return <SoundContext value={{ enabled, toggle }}>{children}</SoundContext>
}
export const useSounds = () => useContext(SoundContext)
export async function outcome(result: Promise<unknown> | undefined) {
  if (!result) return
  const ok = await result
  if (ok !== undefined) play(ok ? "success" : "error", { emphasis: "subtle" })
}
