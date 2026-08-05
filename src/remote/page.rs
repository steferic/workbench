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
<!-- Deliberately not `black-translucent`. Under it iOS gives a home-screen
     app a web view the full width but the height of the screen *minus* the
     status bar, pinned to the top — so the page ends one status-bar-height
     above the bottom of the screen and nothing the page can paint reaches
     that strip. `default` lets iOS size the view properly; the status bar
     takes its colour from `theme-color`, which the theme toggle keeps in
     step with the header. -->
<meta name="apple-mobile-web-app-status-bar-style" content="default">
<meta name="theme-color" content="#171a21">
<title>workbench</title>
<style>
  /* One palette per line, twice over: CSS cannot name a set of custom
     properties and apply it from two selectors, and the toggle needs to beat
     the system preference in both directions. */
  :root {
    color-scheme:dark;
    --bg:#0c0e13; --surface:#171a21; --raised:#1e222b; --line:#272c37;
    --fg:#e8eaf0; --dim:#8b93a4; --faint:#5d6575;
    --accent:#4361ee; --on-accent:#fff;
    --warn:#f0b429; --warn-bg:#2a2113; --ok:#57c785;
    --shadow:0 1px 2px #0000004d;
  }
  @media (prefers-color-scheme: light) {
    :root:not([data-theme="dark"]) {
      color-scheme:light;
      --bg:#f2f3f7; --surface:#fff; --raised:#fff; --line:#e3e6ec;
      --fg:#14161b; --dim:#5f6779; --faint:#98a0b0;
      --accent:#3355e8; --on-accent:#fff;
      --warn:#a96a00; --warn-bg:#fdf5e6; --ok:#1f8a4c;
      --shadow:0 1px 2px #10121a14, 0 1px 1px #10121a0f;
    }
  }
  :root[data-theme="light"] {
    color-scheme:light;
    --bg:#f2f3f7; --surface:#fff; --raised:#fff; --line:#e3e6ec;
    --fg:#14161b; --dim:#5f6779; --faint:#98a0b0;
    --accent:#3355e8; --on-accent:#fff;
    --warn:#a96a00; --warn-bg:#fdf5e6; --ok:#1f8a4c;
    --shadow:0 1px 2px #10121a14, 0 1px 1px #10121a0f;
  }
  * { box-sizing:border-box; -webkit-tap-highlight-color:transparent; }
  /* The composer's colour, so the strip under the home indicator reads as
     part of it rather than as a black gap below the page. */
  html { height:100%; overflow:hidden; background:var(--surface); }
  body {
    /* Pinned to the viewport's edges, and *only* that. There is deliberately
       no height here: with `top`, `bottom` and `height` all set, CSS drops
       `bottom` and the height wins — so the `height:100dvh` that used to sit
       here as a "fallback" was silently cancelling the pinning, and the page
       ended exactly one under-reported viewport short of the bottom. That was
       the footer gap.

       `inset:0` is right in both modes a home-screen app runs in: with
       `viewport-fit=cover` the containing block is the whole screen, and
       without it the containing block is the web view, which already excludes
       the status bar. Either way the composer reaches the bottom edge. */
    position:fixed; inset:0; overflow:hidden;
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
  .tree { flex:1; overflow-y:auto; padding-bottom:8px; }
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
  .server {
    display:flex; align-items:center; gap:9px; width:100%; text-align:left;
    padding:8px 16px 8px 38px; color:var(--fg); font-size:14px;
    text-decoration:none;
  }
  .server .port {
    font:12px/1 ui-monospace,SFMono-Regular,Menlo,monospace; color:var(--accent);
    font-weight:600; flex:none;
  }
  .server .cmd { color:var(--dim); font-size:12px; margin-left:auto; flex:none; }
  .new { display:flex; gap:8px; padding:2px 16px 12px 38px; }
  .new button {
    flex:1; font-size:13px; padding:8px; border-radius:10px;
    border:1px dashed var(--line); background:none; color:var(--dim);
  }
  .notify {
    flex:none; margin:8px 16px 0; padding:11px 13px; text-align:left;
    border:1px solid var(--line); border-radius:11px; background:var(--bg);
    color:var(--fg); font-size:13.5px; line-height:1.35;
  }
  .notify.on { border-color:var(--ok); color:var(--dim); }
  .theme {
    flex:none; display:flex; gap:2px; margin:8px 16px;
    margin-bottom:calc(16px + env(safe-area-inset-bottom));
    padding:3px; border-radius:11px; background:var(--bg); border:1px solid var(--line);
  }
  .theme button {
    flex:1; padding:8px 0; border:0; border-radius:8px; background:none;
    color:var(--dim); font-size:13px; font-weight:600;
  }
  .theme button.on { background:var(--surface); color:var(--fg); box-shadow:var(--shadow); }

  /* ?debug=1 — what the browser thinks the viewport is. */
  .debug {
    position:fixed; left:8px; bottom:8px; z-index:99; margin:0;
    background:#000000d9; color:#7CFF9B; border-radius:8px; padding:7px 9px;
    font:10px/1.35 ui-monospace,SFMono-Regular,Menlo,monospace; white-space:pre;
    pointer-events:none;
  }
</style>
<script>
  /* Before the first paint, so a chosen theme never flashes the other one. */
  (function () {
    var picked = localStorage.getItem("theme");
    if (picked === "light" || picked === "dark") {
      document.documentElement.setAttribute("data-theme", picked);
    }
  })();
</script>
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
  <button class="notify" id="notify" onclick="enablePush()">
    <span>Notify me when an agent is blocked</span>
  </button>
  <div class="theme" id="theme">
    <button onclick="setTheme('system')">Auto</button>
    <button onclick="setTheme('light')">Light</button>
    <button onclick="setTheme('dark')">Dark</button>
  </div>
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
let thread = [];                        // the open conversation, accumulated
let have = 0;                           // how much of it we hold (see msg_total)
let current = store.get("agent", null); // the conversation on screen
let sent = [];                          // your messages, until the journal catches up
let busy = false;
let signature = "";                     // what the log currently shows
let tag = null;                         // ETag of the last snapshot we took

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

/* Fold a response into the conversation we hold. The server sends only what
   we said we were missing, so this is normally a no-op — and when we are
   further behind than its window, it says so and we take its copy instead.

   Only ever the agent on screen: a snapshot fetched in the moment between
   asking for a different conversation and the desktop switching to it still
   describes the old one. */
function merge(a) {
  if (!a) return;
  if (a.msg_reset) thread = a.messages;
  else if (a.messages.length) thread = thread.concat(a.messages);
  if (a.msg_total) have = a.msg_total;
}

function pick(id) {
  current = id;
  store.set("agent", id);
  sent = [];
  thread = [];
  have = 0;
  // The ETag says "same as the body you already folded in". Having just
  // thrown that away, a 304 would leave the log empty.
  tag = null;
  signature = "";
  post("/api/focus", { agent: id });   // only the open conversation is published
  if (drawerOpen) toggleDrawer();
  render();
  refresh();                           // don't sit on an empty log for a tick
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

/* A readout of the viewport as the browser sees it.
   
   Reached by tapping the agent's name three times, *not* only by `?debug=1`:
   an app on the home screen has no URL bar, so a query parameter is a
   diagnostic you cannot get at from the one place the bug appears. */
function debugOn() {
  return new URLSearchParams(location.search).get("debug") === "1"
      || store.get("debug", "") === "1";
}

function toggleDebug() {
  const on = !debugOn();
  store.set("debug", on ? "1" : "0");
  if (!on) document.querySelector(".debug")?.remove();
  showDebug();
  note(on ? "viewport readout on — tap the name 3× to hide" : "readout off");
}

let taps = 0;
let tapsExpire = null;
document.querySelector("header .who").addEventListener("click", () => {
  clearTimeout(tapsExpire);
  tapsExpire = setTimeout(() => { taps = 0; }, 900);
  if (++taps >= 3) { taps = 0; toggleDebug(); }
});

function showDebug() {
  if (!debugOn()) return;
  let box = document.querySelector(".debug");
  if (!box) {
    box = document.createElement("pre");
    box.className = "debug";
    document.body.appendChild(box);
  }
  // env() is only readable through something that has been laid out with it.
  const probe = document.createElement("div");
  probe.style.cssText = "position:fixed;visibility:hidden;padding:" +
    "env(safe-area-inset-top) env(safe-area-inset-right) " +
    "env(safe-area-inset-bottom) env(safe-area-inset-left)";
  document.body.appendChild(probe);
  const inset = getComputedStyle(probe);
  const body = document.body.getBoundingClientRect();
  const html = document.documentElement.getBoundingClientRect();
  const composer = document.querySelector(".composer").getBoundingClientRect();
  const short = c => (c || "").replace(/rgba?\(|\)|\s/g, "");

  box.textContent = [
    `screen     ${screen.width}x${screen.height}`,
    `innerH     ${innerHeight}`,
    `visualVP   ${Math.round(visualViewport ? visualViewport.height : 0)}`,
    `docClient  ${document.documentElement.clientHeight}`,
    `html       ${Math.round(html.height)} @top ${Math.round(html.top)}`,
    `body       ${Math.round(body.height)} @top ${Math.round(body.top)}`,
    `composer   ends ${Math.round(composer.bottom)}`,
    `GAP        ${Math.round(innerHeight - composer.bottom)}`,
    // Which element is painting the strip below the composer settles whether
    // the body is short or the composer is not at its bottom.
    `paints     html ${short(getComputedStyle(document.documentElement).backgroundColor)}`,
    `           body ${short(getComputedStyle(document.body).backgroundColor)}`,
    `           at-gap ${short(atGap())}`,
    `safe-area  top ${inset.paddingTop} bottom ${inset.paddingBottom}`,
    `standalone ${navigator.standalone === true}`,
  ].join("\n");
  probe.remove();
}

/* What is actually underneath a point in the dead strip. */
function atGap() {
  const y = Math.round(innerHeight - 6);
  const el = document.elementFromPoint(Math.round(innerWidth / 2), y);
  if (!el) return "nothing (outside the page)";
  return el.tagName.toLowerCase() + "." + (el.className || "-") + " " +
         getComputedStyle(el).backgroundColor;
}

/* ---- notifications ---------------------------------------------------- */

/* iOS only allows this for a web app on the home screen, and only over https,
   and only from a tap — so it is a button rather than something done on load,
   and it says which of those is missing rather than failing quietly. */
async function enablePush() {
  const button = document.getElementById("notify");
  const say = text => { button.querySelector("span").textContent = text; };

  if (!("serviceWorker" in navigator) || !("PushManager" in window)) {
    say(window.isSecureContext
      ? "This browser cannot do notifications. On iOS, add the page to your home screen first."
      : "Notifications need https — run: tailscale serve --bg 8765");
    return;
  }
  try {
    if (await Notification.requestPermission() !== "granted") {
      say("Notifications refused. Allow them in Settings to turn this on.");
      return;
    }
    const registration = await navigator.serviceWorker.register(q("/sw.js"));
    await navigator.serviceWorker.ready;

    let subscription = await registration.pushManager.getSubscription();
    if (!subscription) {
      const key = (await (await fetch(q("/api/push-key"))).text()).trim();
      subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: keyBytes(key),
      });
    }
    // The endpoint is the whole address; nothing else here is needed, since
    // the push carries no payload to encrypt.
    await post("/api/subscribe", { agent: "-", text: subscription.endpoint });
    store.set("push", "on");
    markPush(true);
  } catch (err) {
    say("Could not turn them on: " + err.message);
  }
}

/* base64url → the Uint8Array `subscribe` insists on. */
function keyBytes(key) {
  const padded = (key + "=".repeat((4 - key.length % 4) % 4)).replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(padded);
  return Uint8Array.from([...raw].map(c => c.charCodeAt(0)));
}

function markPush(on) {
  const button = document.getElementById("notify");
  button.classList.toggle("on", on);
  button.querySelector("span").textContent = on
    ? "Notifying this device when an agent is blocked"
    : "Notify me when an agent is blocked";
}

/* ---- theme ------------------------------------------------------------ */

/* "system" follows the phone; the other two override it until you say
   otherwise. The status bar is told separately: on iOS it is chrome, not
   page, and only `theme-color` reaches it. */
function setTheme(mode) {
  if (mode === "system") localStorage.removeItem("theme");
  else localStorage.setItem("theme", mode);

  if (mode === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", mode);

  const surface = getComputedStyle(document.documentElement).getPropertyValue("--surface").trim();
  document.querySelector('meta[name="theme-color"]').setAttribute("content", surface);

  const buttons = document.querySelectorAll("#theme button");
  ["system", "light", "dark"].forEach((name, i) => buttons[i].classList.toggle("on", name === mode));
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
  for (const m of thread) {
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
  if (!thread.length && a.tail.length) {
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
    // Dev servers running in this project, reachable on the tailnet at the
    // same port they use locally.
    const servers = open ? p.servers.map(s => `
      <a class="server" href="${esc(s.url)}" target="_blank" rel="noopener">
        <span class="port">:${s.port}</span>
        <span>${esc(s.url.replace(/^https?:\/\//, ""))}</span>
        <span class="cmd">${esc(s.command)}</span>
      </a>`).join("") : "";
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
      </button>${rows}${servers}${add}`;
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
  const last = thread[thread.length - 1];
  const next = [current, a.status, thread.length, last?.text.length || 0,
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
    // `have` asks for only the messages we do not hold; the ETag turns a
    // tick where nothing moved at all into an empty 304.
    const res = await fetch(q("/api/state?have=" + have), {
      cache: "no-store",
      headers: tag ? { "If-None-Match": tag } : {},
    });
    if (res.status === 304) return;
    if (!res.ok) throw new Error(res.status === 401 ? "bad or missing token" : await res.text());
    tag = res.headers.get("ETag");
    data = await res.json();
  } catch (err) {
    document.getElementById("hwhat").textContent = "offline";
    return;
  }
  merge(agent(current));
  // Drop the local echo once the agent's journal has your message in it.
  const said = thread.filter(m => m.role === "you").map(m => m.text);
  sent = sent.filter(t => !said.some(s => s.startsWith(t.slice(0, 40))));
  render();
}

setTheme(localStorage.getItem("theme") || "system");
markPush(store.get("push", "") === "on");
showDebug();
if (window.visualViewport) visualViewport.addEventListener("resize", showDebug);
setInterval(showDebug, 1000);
if (current) post("/api/focus", { agent: current });
refresh();
setInterval(refresh, 1000);
</script>
</body>
</html>
"##;

/// What iOS reads when you add the page to the home screen. Installing is not
/// cosmetic there: Safari only allows Web Push for a web app on the home
/// screen, so without this there are no notifications.
pub const MANIFEST: &str = r##"{
  "name": "workbench",
  "short_name": "workbench",
  "start_url": "./",
  "scope": "./",
  "display": "standalone",
  "background_color": "#0c0e13",
  "theme_color": "#171a21"
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
      const res = await fetch(url("/api/state?have=999999999"), { cache: "no-store" });
      if (res.ok) {
        const agents = (await res.json()).agents;
        const blocked = agents.filter(a => a.status === "blocked");
        // The push carries no payload, so which kind of news this is has to be
        // read back off the state. A recent finish is the only other reason.
        const finished = agents.filter(a => a.finished_ago !== null && a.finished_ago < 180);

        if (blocked.length === 1) {
          const a = blocked[0];
          tag = "workbench-blocked";
          title = a.provider + " · " + a.project;
          // The whole question, flattened. The useful part is usually the
          // command it wants to run, not the "do you want to proceed?" — so
          // take the lot and let the notification truncate.
          body = a.prompt
            ? a.prompt.lines.map(l => l.trim()).filter(Boolean).join(" · ").slice(0, 180)
            : (a.reason || "is waiting for you");
        } else if (blocked.length > 1) {
          tag = "workbench-blocked";
          title = blocked.length + " agents need you";
          body = blocked.map(a => a.provider + " · " + a.project).join(", ");
        } else if (finished.length === 1) {
          const a = finished[0];
          tag = "workbench-finished";
          title = a.provider + " · " + a.project;
          body = "Finished" + (a.running ? ": " + a.running : "") +
                 (a.queued.length ? " · " + a.queued.length + " still queued" : "");
        } else if (finished.length > 1) {
          tag = "workbench-finished";
          title = finished.length + " agents finished";
          body = finished.map(a => a.provider + " · " + a.project).join(", ");
        } else {
          // Answered or picked up again at the desk between the poke and its
          // delivery — there is nothing left to say.
          return;
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
