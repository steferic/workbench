//! The phone page: a chat app for whichever agent you have open.
//!
//! One self-contained document, no network beyond this host. The conversation
//! is the screen; projects and their agents live in a drawer behind the menu
//! button, so switching agents is two taps and reading one is none.
//!
//! What it renders is the agent's own journal (see `super::thread`) — your
//! turns, its replies, and a slim line per tool call — not a shrunken copy of
//! the terminal. When an agent stops on a question, the question and its real
//! choices are rendered as buttons; tapping one sends that option's key.
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
    --bg:#0c0e13; --surface:#171a21; --raised:#1e222b; --line:#272c37;
    --fg:#e8eaf0; --dim:#8b93a4; --faint:#5d6575;
    --accent:#6d8cff; --on-accent:#0a0c11;
    --warn:#f0b429; --warn-bg:#2a2113; --ok:#57c785;
    --shadow:0 1px 2px #0000004d;
  }
  @media (prefers-color-scheme: light) {
    :root {
      --bg:#f2f3f7; --surface:#fff; --raised:#fff; --line:#e3e6ec;
      --fg:#14161b; --dim:#5f6779; --faint:#98a0b0;
      --accent:#3355e8; --on-accent:#fff;
      --warn:#a96a00; --warn-bg:#fdf5e6; --ok:#1f8a4c;
      --shadow:0 1px 2px #10121a14, 0 1px 1px #10121a0f;
    }
  }
  * { box-sizing:border-box; -webkit-tap-highlight-color:transparent; }
  /* The composer's colour, so the strip under the home indicator reads as
     part of it rather than as a black gap below the page. */
  html { height:100%; overflow:hidden; background:var(--surface); }
  body {
    /* 100% is the *large* viewport on iOS: with Safari's bars showing, the
       page runs off the bottom and the composer sits below the fold. dvh is
       what is actually visible, and it follows the bars as they hide. */
    height:100vh; height:100dvh; overflow:hidden;
    margin:0; background:var(--bg); color:var(--fg);
    display:flex; flex-direction:column;
    font:15px/1.5 -apple-system,ui-sans-serif,system-ui,"Segoe UI",sans-serif;
    -webkit-font-smoothing:antialiased;
  }
  button { font:inherit; color:inherit; }

  /* ---- header ---------------------------------------------------------- */
  header {
    flex:none; display:flex; align-items:center; gap:11px;
    padding:max(9px,env(safe-area-inset-top)) 10px 9px 14px;
    background:var(--surface); border-bottom:1px solid var(--line);
  }
  .who { display:flex; flex-direction:column; min-width:0; flex:1; gap:1px; }
  .who b { font-size:16px; font-weight:650; letter-spacing:-.01em; }
  .who span {
    font-size:12.5px; color:var(--dim);
    overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  }
  .dot { width:9px; height:9px; border-radius:50%; background:var(--faint); flex:none; }
  .dot.blocked { background:var(--warn); box-shadow:0 0 0 3px #f0b4292e; }
  .dot.working { background:var(--accent); animation:pulse 1.5s ease-in-out infinite; }
  .dot.idle { background:var(--ok); }
  @keyframes pulse { 50% { opacity:.3; } }

  .icon {
    flex:none; width:38px; height:38px; border-radius:11px; position:relative;
    display:grid; place-items:center; font-size:16px;
    background:none; border:1px solid transparent;
  }
  .icon:active { background:var(--raised); }
  .icon .badge {
    position:absolute; top:2px; right:1px; min-width:16px; height:16px; padding:0 4px;
    background:var(--warn); color:#1a1305; border-radius:999px;
    font-size:10px; font-weight:700; line-height:16px; text-align:center;
  }

  /* ---- conversation ---------------------------------------------------- */
  #log { flex:1; overflow-y:auto; -webkit-overflow-scrolling:touch; padding:14px 12px 6px; }
  /* A flex row per message, so a bubble hugs its text but a wide code block
     inside one cannot shrink it to a column of single letters. */
  .row { display:flex; margin-bottom:9px; }
  .row.you { justify-content:flex-end; }
  .msg {
    max-width:86%; min-width:0; padding:9px 13px;
    border-radius:19px 19px 19px 6px; box-shadow:var(--shadow);
    background:var(--surface); white-space:pre-wrap; overflow-wrap:anywhere;
  }
  .row.you .msg {
    background:var(--accent); color:var(--on-accent);
    border-radius:19px 19px 6px 19px;
  }
  .row.pending .msg { opacity:.55; }
  .msg code {
    font:12.5px/1.45 ui-monospace,SFMono-Regular,Menlo,monospace;
    background:#8b93a426; border-radius:5px; padding:1px 4px;
  }
  .msg pre {
    margin:7px 0 3px; padding:9px 11px; border-radius:11px; background:var(--bg);
    max-width:100%; overflow-x:auto; -webkit-overflow-scrolling:touch;
  }
  .msg pre code { background:none; padding:0; font-size:12px; }
  .row.you .msg pre { background:#0000002e; }

  /* A tool call is not speech: one dim line, so the conversation does not
     look like it skipped a beat. */
  .tool {
    display:flex; align-items:baseline; gap:7px; margin:0 2px 7px; color:var(--dim);
    font-size:12.5px; min-width:0;
  }
  .tool .n { font-weight:600; flex:none; }
  .tool .d {
    font:11.5px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace; color:var(--faint);
    overflow:hidden; text-overflow:ellipsis; white-space:nowrap; min-width:0;
  }
  .when { text-align:center; color:var(--faint); font-size:11.5px; margin:12px 0 10px; }
  .raw {
    font:11px/1.4 ui-monospace,SFMono-Regular,Menlo,monospace; color:var(--dim);
    white-space:pre-wrap; overflow-wrap:anywhere;
  }
  .typing { display:flex; gap:4px; padding:11px 14px 4px; }
  .typing i {
    width:6px; height:6px; border-radius:50%; background:var(--faint);
    animation:blink 1.3s infinite;
  }
  .typing i:nth-child(2) { animation-delay:.18s; }
  .typing i:nth-child(3) { animation-delay:.36s; }
  @keyframes blink { 0%,60%,100% { opacity:.25; } 30% { opacity:1; } }
  .empty { color:var(--dim); text-align:center; padding:48px 20px; font-size:14px; }

  /* ---- the question an agent is stopped on ----------------------------- */
  .ask {
    flex:none; margin:0 10px 8px; padding:12px 13px 11px;
    background:var(--warn-bg); border:1px solid var(--warn); border-radius:16px;
  }
  .ask h3 {
    margin:0 0 8px; font-size:11px; font-weight:700; letter-spacing:.12em;
    text-transform:uppercase; color:var(--warn);
  }
  .ask .body {
    font:12px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace; color:var(--fg);
    white-space:pre-wrap; overflow-wrap:anywhere; max-height:33vh; overflow-y:auto;
    margin-bottom:11px;
  }
  .ask button {
    display:block; width:100%; text-align:left; margin-top:7px; padding:11px 13px;
    border-radius:12px; border:1px solid var(--line); background:var(--surface);
    font-size:14px; line-height:1.35;
  }
  .ask button.first { background:var(--accent); border-color:var(--accent); color:var(--on-accent); font-weight:600; }
  .ask button:active { transform:scale(.985); }
  .ask .key { opacity:.55; font-weight:600; margin-right:7px; }

  /* ---- composer -------------------------------------------------------- */
  .composer {
    flex:none; display:flex; gap:8px; align-items:flex-end;
    padding:8px 10px calc(8px + env(safe-area-inset-bottom));
    background:var(--surface); border-top:1px solid var(--line);
  }
  textarea {
    flex:1; min-width:0; resize:none; max-height:132px;
    padding:9px 14px; border-radius:20px; border:1px solid var(--line);
    background:var(--bg); color:var(--fg);
    /* 16px keeps iOS from zooming the page when the field takes focus. */
    font-family:inherit; font-size:16px; line-height:1.4;
  }
  textarea:focus { outline:none; border-color:var(--accent); }
  .act {
    flex:none; width:40px; height:40px; border-radius:50%; border:1px solid var(--line);
    background:var(--bg); display:grid; place-items:center; font-size:16px;
  }
  .act.send { background:var(--accent); border-color:var(--accent); color:var(--on-accent); font-size:19px; }
  .act.send:disabled { opacity:.35; }
  .act.on { background:var(--warn); border-color:var(--warn); color:#1a1305; }
  .note {
    flex:none; color:var(--dim); font-size:12.5px; text-align:center;
    padding:0 14px 7px;
  }

  /* ---- drawer ---------------------------------------------------------- */
  .scrim {
    position:fixed; inset:0; background:#00000073; opacity:0; pointer-events:none;
    transition:opacity .2s ease; backdrop-filter:blur(1px);
  }
  .scrim.open { opacity:1; pointer-events:auto; }
  aside {
    position:fixed; top:0; right:0; bottom:0; width:min(87vw,352px);
    background:var(--surface); border-left:1px solid var(--line);
    transform:translateX(100%); transition:transform .22s cubic-bezier(.32,.72,0,1);
    display:flex; flex-direction:column; padding-top:env(safe-area-inset-top);
  }
  aside.open { transform:none; }
  aside h2 {
    font-size:11px; letter-spacing:.14em; text-transform:uppercase; color:var(--dim);
    margin:16px 18px 6px; font-weight:700;
  }
  .tree { overflow-y:auto; padding-bottom:calc(20px + env(safe-area-inset-bottom)); }
  .proj, .agent {
    display:flex; align-items:center; gap:10px; width:100%; text-align:left;
    background:none; border:0; padding:12px 16px;
  }
  .proj { font-weight:600; }
  .proj .caret { color:var(--faint); font-size:10px; width:10px; }
  .proj .name { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .proj .n { margin-left:auto; color:var(--faint); font-size:12px; font-weight:400; }
  .agent { padding:10px 16px 10px 38px; }
  .agent .label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .agent.current { background:var(--raised); }
  .agent .what { color:var(--dim); font-size:12px; margin-left:auto; flex:none; }
  .pill {
    background:var(--warn); color:#1a1305; border-radius:999px;
    font-size:10px; font-weight:700; padding:1px 6px;
  }
  .new { display:flex; gap:8px; padding:2px 16px 12px 38px; }
  .new button {
    flex:1; font-size:13px; padding:8px; border-radius:10px;
    border:1px dashed var(--line); background:none; color:var(--dim);
  }
</style>
</head>
<body>
<header>
  <span class="dot" id="hdot"></span>
  <span class="who"><b id="hname">—</b><span id="hwhat"></span></span>
  <button class="icon" id="cycle" onclick="cycleAgent()" title="next agent here" hidden>⇄</button>
  <button class="icon" onclick="toggleDrawer()" title="projects">☰<span class="badge" id="hbadge" hidden></span></button>
</header>

<div id="log"><div class="empty">connecting…</div></div>
<div id="ask"></div>
<div class="note" id="note" hidden></div>

<div class="composer">
  <button class="act" id="queue" onclick="queueMessage()" title="add to this agent's queue">＋</button>
  <textarea id="msg" rows="1" placeholder="Message"></textarea>
  <button class="act" id="mic" onclick="toggleMic()" title="dictate">🎤</button>
  <button class="act send" id="send" onclick="sendMessage()" disabled>↑</button>
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
let sent = [];                          // your messages, until the journal catches up
let busy = false;
let signature = "";                     // what the log currently shows

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
  if (text) setTimeout(() => { el.hidden = true; }, 4000);
}

function agent(id) { return (data?.agents || []).find(a => a.id === id) || null; }

function pick(id) {
  current = id;
  store.set("agent", id);
  sent = [];
  signature = "";
  post("/api/focus", { agent: id });   // only the open conversation is published
  if (drawerOpen) toggleDrawer();
  render();
}

/* Next agent in the same project, wrapping — for flicking between the two or
   three agents you have on one thing. */
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

function take() {
  const box = document.getElementById("msg");
  const text = box.value.trim();
  if (!text || !current) return null;
  box.value = "";
  box.style.height = "auto";
  document.getElementById("send").disabled = true;
  return text;
}

function sendMessage() {
  const text = take();
  if (!text) return;
  sent.push(text);                      // appears immediately, like a chat app
  render();
  post("/api/reply", { agent: current, text });
}

/* The queue is the other way to give an agent work: it waits for the turn in
   flight to end instead of interrupting it. */
function queueMessage() {
  const text = take();
  if (!text) return;
  post("/api/todo", { agent: current, text });
  note("queued — it goes out when this turn ends");
}

function answer(key) {
  post("/api/answer", { agent: current, text: key });
  document.getElementById("ask").innerHTML = "";
}

const box = document.getElementById("msg");
box.addEventListener("input", e => {
  e.target.style.height = "auto";
  e.target.style.height = Math.min(e.target.scrollHeight, 132) + "px";
  document.getElementById("send").disabled = !e.target.value.trim();
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

function toggleProject(id) {
  const key = "proj:" + id;
  store.set(key, store.get(key, "1") === "1" ? "0" : "1");
  renderTree();
}

/* ---- rendering ------------------------------------------------------- */

/* Agents write markdown. Escaping happens first, so nothing here can inject:
   these patterns only ever wrap text that is already inert. */
function markdown(text) {
  let html = esc(text);
  // The blank lines around a fence are markdown's spacing; the block has its
  // own margins, and pre-wrap would otherwise render them as a gap.
  html = html.replace(/\n*```[a-z]*\n([\s\S]*?)```\n*/g, (_, code) => "<pre><code>" + code.replace(/\n$/, "") + "</code></pre>");
  html = html.replace(/`([^`\n]+)`/g, "<code>$1</code>");
  html = html.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
  return html;
}

const clock = at => {
  const d = new Date(at);
  return isNaN(d) ? "" : d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
};

function messagesHtml(a) {
  const parts = [];
  let last = null;
  for (const m of a.messages) {
    // A gap means the conversation was picked up later; say when.
    const at = m.at ? new Date(m.at) : null;
    if (at && (!last || at - last > 10 * 60 * 1000)) parts.push('<div class="when">' + clock(m.at) + "</div>");
    if (at) last = at;

    if (m.role === "tool") {
      const [name, detail] = m.text.split(" · ");
      parts.push('<div class="tool"><span class="n">' + esc(name) + '</span>' +
                 '<span class="d">' + esc(detail || "") + "</span></div>");
    } else {
      parts.push('<div class="row ' + (m.role === "you" ? "you" : "") + '">' +
                 '<div class="msg">' + markdown(m.text) + "</div></div>");
    }
  }
  if (!a.messages.length && a.tail.length) {
    // No journal we can read: the terminal is all there is.
    parts.push('<div class="raw">' + esc(a.tail.join("\n")) + "</div>");
  }
  for (const t of sent) parts.push('<div class="row you pending"><div class="msg">' + esc(t) + "</div></div>");
  if (a.status === "working") parts.push('<div class="typing"><i></i><i></i><i></i></div>');
  if (!parts.length) {
    parts.push('<div class="empty">' +
      (a.status === "stopped" ? "This agent is stopped. Send a message to wake it."
                              : "Nothing said yet. Say something.") + "</div>");
  }
  return parts.join("");
}

function askHtml(a) {
  if (!a.prompt) return "";
  const options = a.prompt.options.map((o, i) =>
    '<button class="' + (i === 0 ? "first" : "") + '" onclick="answer(\'' + esc(o.key) + '\')">' +
    '<span class="key">' + esc(o.key) + "</span>" + esc(o.label) + "</button>").join("");
  return '<div class="ask"><h3>waiting on you</h3>' +
    '<div class="body">' + esc(a.prompt.lines.join("\n")) + "</div>" + options + "</div>";
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
        <span class="label">${esc(a.provider)}</span>
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
        <span class="caret">${open ? "▼" : "▶"}</span>
        <span class="name">${esc(p.name)}</span>
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
  document.getElementById("hwhat").textContent = [
    a.project,
    a.running || (a.queued.length ? a.queued.length + " queued" : null),
    a.holding,
  ].filter(Boolean).join(" · ");
  document.getElementById("hdot").className = "dot " + a.status;
  document.getElementById("cycle").hidden =
    data.agents.filter(x => x.project_id === a.project_id).length < 2;

  // Redraw only when there is something new: rewriting the log every second
  // fights your scrolling and drops any text you had selected.
  const last = a.messages[a.messages.length - 1];
  const next = [current, a.status, a.messages.length, last?.text.length || 0,
                a.tail.length, sent.length].join("|");
  if (next !== signature) {
    signature = next;
    const log = document.getElementById("log");
    // Stay put if you scrolled up to read; follow along if you were at the end.
    const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 80;
    log.innerHTML = messagesHtml(a);
    if (atBottom) log.scrollTop = log.scrollHeight;
  }
  document.getElementById("ask").innerHTML = askHtml(a);
}

async function refresh() {
  // Only a POST holds this off, so its effect is in the next snapshot rather
  // than racing it. Typing no longer does: the composer is not redrawn, and
  // freezing the conversation the moment the keyboard opens is worse than
  // anything it was protecting against.
  if (busy) return;
  try {
    const res = await fetch(q("/api/state"), { cache: "no-store" });
    if (!res.ok) throw new Error(res.status === 401 ? "bad or missing token" : await res.text());
    data = await res.json();
  } catch (err) {
    document.getElementById("hwhat").textContent = "offline";
    return;
  }
  // Drop the local echo once the agent's journal has your message in it.
  const said = (agent(current)?.messages || []).filter(m => m.role === "you").map(m => m.text);
  sent = sent.filter(t => !said.some(s => s.startsWith(t.slice(0, 40))));
  render();
}

if (current) post("/api/focus", { agent: current });
refresh();
setInterval(refresh, 1000);
</script>
</body>
</html>
"##;
