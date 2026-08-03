//! The phone page: a chat app for whichever agent you have open.
//!
//! One self-contained document, no network beyond this host. The conversation
//! is the screen; projects and their agents live in a drawer behind the menu
//! button, so switching agents is two taps and reading one is none.
//!
//! Dictation uses the browser's speech recognition, which browsers only allow
//! in a secure context. Over plain http the button says so instead of failing
//! silently — `tailscale serve` puts real HTTPS in front and it starts
//! working. The keyboard's own mic key works either way.

pub const HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover,maximum-scale=1">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<title>workbench</title>
<style>
  :root {
    --bg:#0d0f12; --panel:#16191f; --line:#262b33; --fg:#e6e9ef; --dim:#8a93a3;
    --accent:#7aa2f7; --warn:#e0af68; --ok:#9ece6a; --me:#28324a;
  }
  @media (prefers-color-scheme: light) {
    :root {
      --bg:#f5f6f8; --panel:#fff; --line:#e2e5ea; --fg:#1a1d23; --dim:#667085;
      --accent:#3b5bdb; --warn:#b45309; --ok:#2f9e44; --me:#dbe4ff;
    }
  }
  * { box-sizing:border-box; -webkit-tap-highlight-color:transparent; }
  html, body { height:100%; overflow:hidden; }
  body {
    margin:0; background:var(--bg); color:var(--fg); display:flex; flex-direction:column;
    font:16px/1.45 ui-sans-serif,-apple-system,system-ui,sans-serif;
  }

  header {
    display:flex; align-items:center; gap:10px; flex:none;
    padding:max(10px,env(safe-area-inset-top)) 12px 10px;
    border-bottom:1px solid var(--line); background:var(--panel);
  }
  .title { display:flex; flex-direction:column; min-width:0; }
  .title b { font-size:15px; }
  .title span { font-size:12px; color:var(--dim); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .dot { width:9px; height:9px; border-radius:50%; background:var(--dim); flex:none; }
  .dot.blocked { background:var(--warn); }
  .dot.working { background:var(--accent); animation:pulse 1.4s ease-in-out infinite; }
  .dot.idle { background:var(--ok); }
  @keyframes pulse { 50% { opacity:.35; } }
  .menu { margin-left:6px; background:none; border:1px solid var(--line); color:var(--fg);
          border-radius:10px; padding:8px 11px; font-size:15px; position:relative; }
  .menu:first-of-type { margin-left:auto; }
  .menu .badge {
    position:absolute; top:-5px; right:-5px; background:var(--warn); color:#0b0d10;
    border-radius:999px; font-size:10px; font-weight:700; padding:1px 5px;
  }

  /* Small on purpose: agent output is wide, and seeing more of it at once
     beats comfortable reading of three lines. */
  #log {
    flex:1; overflow-y:auto; -webkit-overflow-scrolling:touch; padding:10px 11px;
    font:10.5px/1.35 ui-monospace,SFMono-Regular,Menlo,monospace; white-space:pre-wrap;
    word-break:break-word;
  }
  #log .sent {
    background:var(--me); border-radius:12px 12px 4px 12px; padding:7px 10px;
    margin:9px 0 9px auto; max-width:85%; width:fit-content; white-space:pre-wrap;
    font:13px/1.35 ui-sans-serif,-apple-system,system-ui,sans-serif;
  }
  .prompt {
    margin:10px 12px; padding:10px 12px; border:1px solid var(--warn); border-radius:12px;
    color:var(--warn); font:14px/1.4 ui-sans-serif,-apple-system,system-ui,sans-serif;
  }

  .composer {
    flex:none; display:flex; gap:7px; align-items:flex-end; background:var(--panel);
    padding:9px 10px calc(9px + env(safe-area-inset-bottom)); border-top:1px solid var(--line);
  }
  textarea, button.act, .row button {
    font:inherit; border-radius:12px; border:1px solid var(--line);
    background:var(--bg); color:var(--fg); padding:10px 12px;
  }
  textarea { flex:1; min-width:0; resize:none; max-height:120px; font-size:16px; }
  button.act { flex:none; font-weight:600; }
  button.act.on { background:var(--warn); border-color:var(--warn); color:#0b0d10; }
  button.act.send { background:var(--accent); border-color:var(--accent); color:#0b0d10; }
  .row { display:flex; gap:7px; margin-top:9px; }
  .row button { flex:1; font-weight:600; }
  .row button.primary { background:var(--accent); border-color:var(--accent); color:#0b0d10; }
  .row button.warn { background:var(--warn); border-color:var(--warn); color:#0b0d10; }

  .scrim {
    position:fixed; inset:0; background:#0006; opacity:0; pointer-events:none;
    transition:opacity .18s ease;
  }
  .scrim.open { opacity:1; pointer-events:auto; }
  aside {
    position:fixed; top:0; right:0; bottom:0; width:min(86vw,340px); background:var(--panel);
    border-left:1px solid var(--line); transform:translateX(100%); transition:transform .2s ease;
    display:flex; flex-direction:column; padding-top:env(safe-area-inset-top);
  }
  aside.open { transform:none; }
  aside h2 {
    font-size:12px; letter-spacing:.14em; text-transform:uppercase; color:var(--dim);
    margin:14px 16px 8px; font-weight:600;
  }
  .tree { overflow-y:auto; padding-bottom:env(safe-area-inset-bottom); }
  .proj, .agent {
    display:flex; align-items:center; gap:9px; width:100%; background:none; border:0;
    color:var(--fg); font:inherit; text-align:left; padding:11px 16px;
  }
  .proj { font-weight:600; }
  .proj .caret { color:var(--dim); font-size:11px; width:10px; }
  .proj .n { margin-left:auto; color:var(--dim); font-size:12px; font-weight:400; }
  .agent { padding-left:37px; font-size:15px; }
  .agent.current { background:var(--me); }
  .agent .what { color:var(--dim); font-size:12px; margin-left:auto; }
  .pill { background:var(--warn); color:#0b0d10; border-radius:999px; font-size:10px;
          font-weight:700; padding:1px 6px; }
  .new { display:flex; gap:7px; padding:4px 16px 12px 37px; }
  .new button {
    flex:1; font:inherit; font-size:13px; padding:7px 9px; border-radius:9px;
    border:1px dashed var(--line); background:none; color:var(--dim);
  }
  .empty { color:var(--dim); text-align:center; padding:40px 16px; }
  .note { color:var(--dim); font-size:12px; padding:0 12px 8px; }
</style>
</head>
<body>
<header>
  <span class="dot" id="hdot"></span>
  <span class="title"><b id="hname">—</b><span id="hwhat"></span></span>
  <button class="menu" id="cycle" onclick="cycleAgent()" title="next agent in this project" hidden>⇄</button>
  <button class="menu" onclick="toggleDrawer()">☰<span class="badge" id="hbadge" hidden></span></button>
</header>

<div id="log"><div class="empty">connecting…</div></div>
<div id="prompt"></div>
<div class="note" id="note" hidden></div>

<div class="composer">
  <textarea id="msg" rows="1" placeholder="message…"></textarea>
  <button class="act" id="mic" onclick="toggleMic()" title="dictate">🎤</button>
  <button class="act send" onclick="sendMessage()">↑</button>
</div>

<div class="scrim" id="scrim" onclick="toggleDrawer()"></div>
<aside id="drawer">
  <h2>projects</h2>
  <div class="tree" id="tree"></div>
</aside>

<script>
const token = new URLSearchParams(location.search).get("t") || "";
const q = p => p + (p.includes("?") ? "&" : "?") + "t=" + encodeURIComponent(token);
const esc = s => (s||"").replace(/[&<>"]/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
const store = {
  get: (k, d) => { const v = localStorage.getItem(k); return v === null ? d : v; },
  set: (k, v) => localStorage.setItem(k, v),
};

let data = null;                        // last snapshot
let current = store.get("agent", null); // the conversation on screen
let sent = [];                          // your messages, echoed until the agent's output catches up
let busy = false;

async function post(path, body) {
  busy = true;
  try {
    const res = await fetch(q(path), {
      method: "POST", headers: {"Content-Type":"application/json"}, body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(await res.text());
  } catch (err) {
    note("could not reach workbench: " + err);
  } finally {
    busy = false;
  }
}

function note(text) {
  const el = document.getElementById("note");
  el.textContent = text;
  el.hidden = !text;
  if (text) setTimeout(() => { el.hidden = true; }, 5000);
}

function agent(id) { return (data?.agents || []).find(a => a.id === id) || null; }

function pick(id) {
  current = id;
  store.set("agent", id);
  sent = [];
  post("/api/focus", { agent: id });   // only the open conversation gets deep history
  if (drawerOpen) toggleDrawer();
  render();
}

/// Next agent in the same project, wrapping — for flicking between the two or
/// three agents you have on one thing.
function cycleAgent() {
  const here = agent(current);
  if (!here) return;
  const siblings = data.agents.filter(a => a.project_id === here.project_id);
  if (siblings.length < 2) return;
  const at = siblings.findIndex(a => a.id === current);
  pick(siblings[(at + 1) % siblings.length].id);
}

function newAgent(projectId, provider) {
  post("/api/new-agent", { agent: projectId, text: provider });
  note("starting " + provider + "…");
  if (drawerOpen) toggleDrawer();
}

function sendMessage() {
  const box = document.getElementById("msg");
  const text = box.value.trim();
  if (!text || !current) return;
  box.value = "";
  box.style.height = "auto";
  sent.push(text);                     // appears immediately, like a chat app
  render();
  post("/api/reply", { agent: current, text });
}

document.getElementById("msg").addEventListener("input", e => {
  e.target.style.height = "auto";
  e.target.style.height = Math.min(e.target.scrollHeight, 120) + "px";
});

/* dictation — needs a secure context, so say so rather than failing quietly */
const Recognition = window.SpeechRecognition || window.webkitSpeechRecognition;
let recog = null, listening = false;

function toggleMic() {
  if (!Recognition) {
    note(window.isSecureContext
      ? "this browser has no speech recognition — use the keyboard's mic key"
      : "dictation needs https (tailscale serve --bg 8765) — the keyboard's mic key works now");
    return;
  }
  if (listening) { recog.stop(); return; }

  recog = new Recognition();
  recog.continuous = true;
  recog.interimResults = true;
  recog.lang = navigator.language || "en-US";

  const box = document.getElementById("msg");
  const before = box.value ? box.value + " " : "";
  recog.onresult = e => {
    let text = "";
    for (let i = e.resultIndex; i < e.results.length; i++) text += e.results[i][0].transcript;
    box.value = before + text;
    box.dispatchEvent(new Event("input"));
  };
  recog.onerror = e => {
    note(e.error === "not-allowed" ? "microphone permission denied" : "dictation: " + e.error);
    setListening(false);
  };
  recog.onend = () => setListening(false);
  recog.start();
  setListening(true);
}

function setListening(on) {
  listening = on;
  document.getElementById("mic").classList.toggle("on", on);
}

let drawerOpen = false;
function toggleDrawer() {
  drawerOpen = !drawerOpen;
  document.getElementById("drawer").classList.toggle("open", drawerOpen);
  document.getElementById("scrim").classList.toggle("open", drawerOpen);
}

function toggleProject(name) {
  const key = "proj:" + name;
  store.set(key, store.get(key, "1") === "1" ? "0" : "1");
  renderTree();
}

function renderTree() {
  // Every project, so you can start an agent in one that has none.
  const projects = (data?.projects || []).map(p => ({
    ...p, agents: (data.agents || []).filter(a => a.project_id === p.id),
  }));
  // Projects with someone waiting on you float up.
  projects.sort((a, b) =>
    (b.agents.some(x => x.status === "blocked") ? 1 : 0) -
    (a.agents.some(x => x.status === "blocked") ? 1 : 0));

  document.getElementById("tree").innerHTML = projects.map(p => {
    const blocked = p.agents.filter(a => a.status === "blocked").length;
    const open = store.get("proj:" + p.id, "1") === "1";
    const rows = open ? p.agents.map(a => `
      <button class="agent ${a.id === current ? "current" : ""}" onclick="pick('${a.id}')">
        <span class="dot ${a.status}"></span>
        <span>${esc(a.provider)}</span>
        <span class="what">${a.status === "blocked" ? "needs you"
          : a.queued.length ? a.queued.length + " queued" : a.status}</span>
      </button>`).join("") : "";
    const add = open ? `
      <div class="new">
        <button onclick="newAgent('${p.id}','claude')">+ Claude</button>
        <button onclick="newAgent('${p.id}','codex')">+ Codex</button>
      </div>` : "";
    return `
      <button class="proj" onclick="toggleProject('${p.id}')">
        <span class="caret">${open ? "▾" : "▸"}</span>
        <span>${esc(p.name)}</span>
        ${blocked ? '<span class="pill">' + blocked + "</span>" : ""}
        <span class="n">${p.agents.length}</span>
      </button>${rows}${add}`;
  }).join("") || '<div class="empty">no projects</div>';
}

function render() {
  if (!data) return;
  const waiting = data.agents.filter(a => a.status === "blocked").length;
  const badge = document.getElementById("hbadge");
  badge.textContent = waiting;
  badge.hidden = !waiting;

  // Nothing chosen yet: open whoever needs you, else the first agent.
  if (!agent(current)) {
    const first = data.agents.find(a => a.status === "blocked") || data.agents[0];
    if (first) { pick(first.id); return; }
  }
  renderTree();

  const a = agent(current);
  if (!a) {
    document.getElementById("log").innerHTML = '<div class="empty">no agents running</div>';
    return;
  }

  document.getElementById("hname").textContent = a.provider;
  document.getElementById("hwhat").textContent =
    a.project + (a.running ? " · " + a.running : a.holding ? " · " + a.holding : "");
  document.getElementById("hdot").className = "dot " + a.status;
  document.getElementById("cycle").hidden =
    data.agents.filter(x => x.project_id === a.project_id).length < 2;

  // Stay put if you scrolled up to read; follow along if you were at the end.
  const log = document.getElementById("log");
  const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 60;
  log.innerHTML = esc((a.tail || []).join("\n"))
    + sent.map(t => '<div class="sent">' + esc(t) + "</div>").join("");
  if (atBottom) log.scrollTop = log.scrollHeight;

  document.getElementById("prompt").innerHTML = a.status === "blocked" ? `
    <div class="prompt">
      ${esc(a.reason || "waiting for you")}
      <div class="row">
        <button class="primary" onclick="post('/api/approve',{agent:'${a.id}'})">Approve</button>
        <button class="warn" onclick="post('/api/deny',{agent:'${a.id}'})">Deny</button>
      </div>
    </div>` : "";
}

async function refresh() {
  // Never redraw while you are typing: it would eat what is in the box.
  if (busy || document.activeElement?.tagName === "TEXTAREA") return;
  try {
    const res = await fetch(q("/api/state"), { cache: "no-store" });
    if (!res.ok) throw new Error(res.status === 401 ? "bad or missing token" : await res.text());
    data = await res.json();
  } catch (err) {
    document.getElementById("hwhat").textContent = "offline";
    return;
  }
  // Drop the local echo once the agent's own output contains it.
  const tail = (agent(current)?.tail || []).join("\n");
  sent = sent.filter(t => !tail.includes(t.slice(0, 40)));
  render();
}

if (current) post("/api/focus", { agent: current });
refresh();
setInterval(refresh, 1000);
</script>
</body>
</html>
"##;
