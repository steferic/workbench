import type { WorkbenchClient } from "./client"

export async function notifications(client: WorkbenchClient, restore = false) {
  if (
    !window.isSecureContext ||
    !("serviceWorker" in navigator) ||
    !("PushManager" in window) ||
    !("Notification" in window)
  ) {
    if (restore) return false
    throw Error(
      "Use HTTPS and add Workbench to your home screen to enable notifications on iPhone."
    )
  }
  const permission = restore
    ? Notification.permission
    : await Notification.requestPermission()
  if (permission !== "granted") {
    if (restore) return false
    throw Error("Allow notifications in your browser settings to turn them on.")
  }
  const registration = await navigator.serviceWorker.register(
    client.url("/sw.js")
  )
  await navigator.serviceWorker.ready
  let subscription = await registration.pushManager.getSubscription()
  if (!subscription && restore) return false
  if (!subscription) {
    const response = await fetch(client.url("/api/push-key"))
    if (!response.ok)
      throw Error("Could not register notifications. Try again.")
    const key = (await response.text())
      .trim()
      .replace(/-/g, "+")
      .replace(/_/g, "/")
    subscription = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: Uint8Array.from(
        atob(key + "=".repeat((4 - (key.length % 4)) % 4)),
        (c) => c.charCodeAt(0)
      ),
    })
  }
  await client.post("/api/subscribe", {
    agent: "-",
    text: subscription.endpoint,
  })
  try {
    localStorage.setItem("push", "on")
  } catch {}
  return true
}
