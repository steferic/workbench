//! Static React phone app plus the existing token-scoped push worker.
//! Rebuild assets with `cd workbench-mobile && npm ci && npm run bundle`.
//! Generated assets are committed so Cargo builds need no Node or network.

pub const HTML: &str = include_str!("../../assets/mobile/index.html");
include!(concat!(env!("OUT_DIR"), "/mobile_assets.rs"));

pub const MANIFEST: &str = r##"{
  "name": "workbench",
  "short_name": "workbench",
  "start_url": "./",
  "scope": "./",
  "display": "standalone",
  "background_color": "#f7f7f8",
  "theme_color": "#f7f7f8"
}"##;

/// The worker that runs when a notification arrives — with the page closed,
/// which is the whole point.
///
/// The push itself is empty (see `super::push`), so the text is written here
/// from state read at delivery. That is deliberately better than a payload:
/// what matters is what is blocked *now*, not what was blocked when the poke
/// was sent. When the phone is off the tailnet and cannot read anything, it
/// says so in general terms rather than saying nothing.
pub const SERVICE_WORKER: &str = r##"
const token = new URL(self.location).searchParams.get("t") || "";
const url = path => path + (path.includes("?") ? "&" : "?") + "t=" + encodeURIComponent(token);

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", event => event.waitUntil(self.clients.claim()));

self.addEventListener("push", event => {
  event.waitUntil((async () => {
    let title = "An agent needs you";
    let body = "Open workbench to see which.";
    let tag = "workbench";
    try {
      // `have` is nonsense on purpose: we want statuses, not the conversation.
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), 10000);
      let res;
      try { res = await fetch(url("/api/state?agent="), {cache:"no-store", signal:controller.signal});
        if (res.ok) { const state = await res.json(); res = {ok:true, json:async () => state}; }
      } finally { clearTimeout(timeout); }
      if (res.ok) {
        const agents = (await res.json()).agents;
        // Its own copy of the rule in page.js: a service worker shares no
        // scope with the page it woke.
        const named = a => a.model || a.provider;
        const blocked = agents.filter(a => a.status === "blocked");
        // The push carries no payload, so which kind of news this is has to be
        // read back off the state. A recent finish is the only other reason.
        const finished = agents.filter(a => a.finished_ago !== null && a.finished_ago < 180);

        // Also read back off the state: an agent that just went over the
        // memory limit. First, because it is the one reason to be woken that
        // the other rows would hide.
        const heavy = agents.filter(a => a.memory_over_ago != null && a.memory_over_ago < 180);
        const gb = a => (a.memory_mb / 1024).toFixed(1) + " GB";

        if (heavy.length === 1) {
          const a = heavy[0];
          tag = "workbench-memory";
          title = named(a) + " · " + a.project + " uses " + gb(a);
          body = "Over the memory limit. Stop it with: workbench kill " + a.id;
        } else if (heavy.length > 1) {
          tag = "workbench-memory";
          title = heavy.length + " agents are over the memory limit";
          body = heavy.map(a => named(a) + " · " + a.project + " " + gb(a)).join(", ");
        } else if (blocked.length === 1) {
          const a = blocked[0];
          tag = "workbench-blocked";
          title = named(a) + " · " + a.project;
          // The whole question, flattened. The useful part is usually the
          // command it wants to run, not the "do you want to proceed?" — so
          // take the lot and let the notification truncate.
          body = a.prompt
            ? a.prompt.lines.map(l => l.trim()).filter(Boolean).join(" · ").slice(0, 180)
            : (a.reason || "is waiting for you");
        } else if (blocked.length > 1) {
          tag = "workbench-blocked";
          title = blocked.length + " agents need you";
          body = blocked.map(a => named(a) + " · " + a.project).join(", ");
        } else if (finished.length === 1) {
          const a = finished[0];
          tag = "workbench-finished";
          title = named(a) + " · " + a.project;
          body = "Finished" + (a.running ? ": " + a.running : "") +
                 (a.queued.length ? " · " + a.queued.length + " still queued" : "");
        } else if (finished.length > 1) {
          tag = "workbench-finished";
          title = finished.length + " agents finished";
          body = finished.map(a => named(a) + " · " + a.project).join(", ");
        } else {
          title = "Workbench update";
          body = "The agent's status changed. Open workbench for the latest conversation.";
        }
      }
    } catch (err) {
      // Off the tailnet: the generic text above still says enough to act on.
    }
    await self.registration.showNotification(title, {
      body,
      tag,           // one notification per kind, replaced rather than piled up
      renotify: true,
    });
  })());
});

self.addEventListener("notificationclick", event => {
  event.notification.close();
  event.waitUntil((async () => {
    const open = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    for (const client of open) {
      if (client.url.includes(self.location.origin)) return client.focus();
    }
    return self.clients.openWindow(url("/"));
  })());
});
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_app_loads_only_embedded_frontend_assets() {
        assert!(HTML.contains("Workbench"));
        let mut assets = 0;
        for part in HTML.split("\"") {
            if part.starts_with("/_next/static/") {
                // Next also quotes these paths inside its hydration JSON.
                let part = part.trim_end_matches('\\');
                assert!(asset(part).is_some(), "Missing frontend asset: {part}");
                assets += 1;
            }
        }
        assert!(assets > 2, "The export must load its JS, CSS and font");
        assert!(asset("/_next/static/../../index.html").is_none());
        assert!(asset("/api/state").is_none());
        assert!(asset("/index.html").is_none());
    }
}
