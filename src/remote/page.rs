//! The phone page: one self-contained document, no network beyond this host.
//!
//! Deliberately plain. It refreshes once a second, keeps the token from the
//! URL it was opened with, and puts the blocked agent at the top with its
//! actions in thumb reach.

pub const HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<title>workbench</title>
<style>
  :root {
    --bg: #0d0f12; --card: #16191f; --line: #262b33;
    --fg: #e6e9ef; --dim: #8a93a3; --accent: #7aa2f7;
    --warn: #e0af68; --ok: #9ece6a;
  }
  @media (prefers-color-scheme: light) {
    :root {
      --bg: #f5f6f8; --card: #fff; --line: #e2e5ea;
      --fg: #1a1d23; --dim: #667085; --accent: #3b5bdb;
      --warn: #b45309; --ok: #2f9e44;
    }
  }
  * { box-sizing: border-box; -webkit-tap-highlight-color: transparent; }
  body {
    margin: 0; background: var(--bg); color: var(--fg);
    font: 16px/1.45 ui-sans-serif, -apple-system, system-ui, sans-serif;
    padding: max(12px, env(safe-area-inset-top)) 12px calc(24px + env(safe-area-inset-bottom));
  }
  header { display: flex; align-items: baseline; gap: 8px; margin: 4px 2px 14px; }
  h1 { font-size: 15px; letter-spacing: .14em; text-transform: uppercase; margin: 0; color: var(--dim); font-weight: 600; }
  #age { font-size: 12px; color: var(--dim); margin-left: auto; }
  .card {
    background: var(--card); border: 1px solid var(--line); border-radius: 14px;
    padding: 13px 14px; margin-bottom: 11px;
  }
  .card.blocked { border-color: var(--warn); }
  .top { display: flex; align-items: center; gap: 8px; }
  .dot { width: 9px; height: 9px; border-radius: 50%; flex: none; background: var(--dim); }
  .blocked .dot { background: var(--warn); }
  .working .dot { background: var(--accent); animation: pulse 1.4s ease-in-out infinite; }
  .idle .dot { background: var(--ok); }
  @keyframes pulse { 50% { opacity: .35; } }
  .who { font-weight: 600; font-size: 15px; }
  .project { color: var(--dim); font-size: 13px; }
  .reason { color: var(--warn); margin: 9px 0 0; font-size: 14px; }
  .running { margin: 9px 0 0; font-size: 14px; }
  .steps { margin: 6px 0 0; padding: 0; list-style: none; color: var(--dim); font-size: 13px; }
  .steps li::before { content: "○ "; }
  .steps li.doing::before { content: "◐ "; color: var(--accent); }
  .steps li.done::before { content: "✓ "; }
  .queued { margin: 8px 0 0; padding: 0; list-style: none; color: var(--dim); font-size: 13px; }
  .queued li::before { content: "· "; }
  .holding { color: var(--warn); font-size: 12px; margin-top: 7px; }
  pre.tail {
    margin: 9px 0 0; padding: 9px 10px; border-radius: 9px; background: var(--bg);
    border: 1px solid var(--line); color: var(--dim); font-size: 11.5px;
    line-height: 1.35; overflow-x: auto; white-space: pre; max-height: 190px;
  }
  .row { display: flex; gap: 7px; margin-top: 11px; }
  button, input {
    font: inherit; border-radius: 10px; border: 1px solid var(--line);
    background: var(--bg); color: var(--fg); padding: 11px 13px;
  }
  button { flex: 1; font-weight: 600; }
  button.primary { background: var(--accent); border-color: var(--accent); color: #0b0d10; }
  button.warn { background: var(--warn); border-color: var(--warn); color: #0b0d10; }
  input { flex: 1; min-width: 0; }
  .empty { color: var(--dim); text-align: center; padding: 40px 0; }
  .err { color: var(--warn); text-align: center; padding: 10px; font-size: 14px; }
</style>
</head>
<body>
<header><h1>workbench</h1><span id="age"></span></header>
<div id="list"><div class="empty">connecting…</div></div>
<script>
const token = new URLSearchParams(location.search).get("t") || "";
const q = (p) => p + (p.includes("?") ? "&" : "?") + "t=" + encodeURIComponent(token);
const esc = (s) => (s || "").replace(/[&<>"]/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
let busy = false;

async function post(path, body) {
  busy = true;
  try {
    const res = await fetch(q(path), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(await res.text());
    await refresh();
  } catch (err) {
    document.getElementById("list").insertAdjacentHTML(
      "afterbegin", '<div class="err">' + esc(String(err)) + "</div>");
  } finally {
    busy = false;
  }
}

function card(a) {
  const steps = a.steps.map(s => '<li class="' + s.state + '">' + esc(s.text) + "</li>").join("");
  const queued = a.queued.map(t => "<li>" + esc(t) + "</li>").join("");
  return `
  <div class="card ${a.status}">
    <div class="top">
      <span class="dot"></span>
      <span class="who">${esc(a.provider)}</span>
      <span class="project">${esc(a.project)}</span>
    </div>
    ${a.reason && a.status === "blocked" ? '<p class="reason">' + esc(a.reason) + "</p>" : ""}
    ${a.tail && a.tail.length ? '<pre class="tail">' + esc(a.tail.join("\n")) + "</pre>" : ""}
    ${a.status === "blocked" ? `
      <div class="row">
        <button class="primary" onclick="post('/api/approve',{agent:'${a.id}'})">Approve</button>
        <button class="warn" onclick="post('/api/deny',{agent:'${a.id}'})">Deny</button>
      </div>
      <div class="row">
        <input id="r-${a.id}" placeholder="or reply…" enterkeyhint="send">
        <button onclick="send('r-${a.id}','/api/reply','${a.id}')">Send</button>
      </div>` : ""}
    ${a.running ? '<p class="running">▶ ' + esc(a.running) + "</p>" : ""}
    ${steps ? '<ul class="steps">' + steps + "</ul>" : ""}
    ${queued ? '<ul class="queued">' + queued + "</ul>" : ""}
    ${a.holding ? '<div class="holding">' + esc(a.holding) + "</div>" : ""}
    <div class="row">
      <input id="t-${a.id}" placeholder="queue a TODO…" enterkeyhint="send">
      <button onclick="send('t-${a.id}','/api/todo','${a.id}')">Add</button>
    </div>
  </div>`;
}

function send(inputId, path, agent) {
  const el = document.getElementById(inputId);
  const text = el.value.trim();
  if (!text) return;
  el.value = "";
  post(path, { agent, text });
}

async function refresh() {
  // Never redraw while a field is focused: it would eat what you are typing.
  if (busy || document.activeElement?.tagName === "INPUT") return;
  let data;
  try {
    const res = await fetch(q("/api/state"), { cache: "no-store" });
    if (!res.ok) throw new Error(res.status === 401 ? "bad or missing token" : await res.text());
    data = await res.json();
  } catch (err) {
    document.getElementById("age").textContent = "offline";
    return;
  }
  document.getElementById("age").textContent =
    new Date(data.at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  document.getElementById("list").innerHTML =
    data.agents.length ? data.agents.map(card).join("") : '<div class="empty">no agents running</div>';
}

refresh();
setInterval(refresh, 1000);
</script>
</body>
</html>
"##;
