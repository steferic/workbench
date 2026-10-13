//! `/words`: the word cloud, served beside the phone page.
//!
//! The layout is a plain spiral placement — biggest word in the middle, each
//! next word walked out along a spiral until it overlaps nothing — done in the
//! page, so the server only ever sends the top words and their counts.

pub const HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>Workbench words</title>
<style>
  @font-face {
    font-family:"IBM Plex Mono"; font-style:normal; font-weight:400;
    font-display:swap; src:url("/font/ibm-plex-mono-400.woff2") format("woff2");
  }
  @font-face {
    font-family:"IBM Plex Mono"; font-style:normal; font-weight:500;
    font-display:swap; src:url("/font/ibm-plex-mono-500.woff2") format("woff2");
  }
  :root {
    color-scheme:dark;
    --bg:#0c0e13; --surface:#171a21; --line:#272c37;
    --fg:#e8eaf0; --dim:#8b93a4; --faint:#5d6575; --accent:#4361ee;
    --c1:#7b93ff; --c2:#5fd3b0; --c3:#f0a35e; --c4:#d58cf0; --c5:#e8eaf0;
  }
  @media (prefers-color-scheme: light) {
    :root:not([data-theme="dark"]) {
      color-scheme:light;
      --bg:#f2f3f7; --surface:#fff; --line:#e3e6ec;
      --fg:#14161b; --dim:#5f6779; --faint:#98a0b0; --accent:#3355e8;
      --c1:#2f4fd6; --c2:#0f8a66; --c3:#b45d12; --c4:#8a3bb0; --c5:#14161b;
    }
  }
  :root[data-theme="light"] {
    color-scheme:light;
    --bg:#f2f3f7; --surface:#fff; --line:#e3e6ec;
    --fg:#14161b; --dim:#5f6779; --faint:#98a0b0; --accent:#3355e8;
    --c1:#2f4fd6; --c2:#0f8a66; --c3:#b45d12; --c4:#8a3bb0; --c5:#14161b;
  }
  * { box-sizing:border-box; }
  body {
    margin:0; background:var(--bg); color:var(--fg);
    font:14px/1.45 "IBM Plex Mono", ui-monospace, monospace;
    padding:max(12px, env(safe-area-inset-top)) 16px max(24px, env(safe-area-inset-bottom));
  }
  main { max-width:960px; margin:0 auto; }
  header { display:flex; align-items:baseline; gap:12px; margin-bottom:12px; }
  header a { color:var(--dim); text-decoration:none; }
  h1 { font-size:18px; font-weight:500; margin:0; flex:1; }
  .status { color:var(--dim); font-size:12px; margin:0 0 12px; min-height:1.4em; }
  .filters { display:flex; flex-wrap:wrap; gap:8px; margin-bottom:12px; }
  .seg { display:flex; border:1px solid var(--line); border-radius:8px; overflow:hidden; }
  .seg button {
    font:inherit; color:var(--dim); background:var(--surface); border:0;
    padding:7px 12px; cursor:pointer;
  }
  .seg button + button { border-left:1px solid var(--line); }
  .seg button[aria-pressed="true"] { background:var(--accent); color:#fff; }
  select {
    font:inherit; color:var(--fg); background:var(--surface);
    border:1px solid var(--line); border-radius:8px; padding:7px 8px; max-width:100%;
  }
  #cloud {
    position:relative; width:100%; height:min(70vh, 560px); overflow:hidden;
    background:var(--surface); border:1px solid var(--line); border-radius:12px;
  }
  #cloud span {
    position:absolute; white-space:nowrap; line-height:1; font-weight:500;
    transform:translate(-50%, -50%); cursor:pointer; user-select:none;
    -webkit-tap-highlight-color:transparent;
    transition:opacity .25s ease, transform .25s ease;
  }
  #cloud span.gone { opacity:0 !important; transform:translate(-50%, -50%) scale(.4); pointer-events:none; }
  .actions { display:flex; flex-wrap:wrap; gap:8px; align-items:center; margin:10px 0 0; }
  .actions button, .chip {
    font:inherit; font-size:13px; color:var(--fg); background:var(--surface);
    border:1px solid var(--line); border-radius:8px; padding:6px 10px; cursor:pointer;
  }
  .actions button:disabled { color:var(--faint); cursor:default; }
  .actions .hint { color:var(--dim); font-size:12px; flex:1 1 100%; }
  #hiddenBox { margin-top:10px; }
  #hiddenBox[hidden] { display:none; }
  #hiddenBox .chips { display:flex; flex-wrap:wrap; gap:6px; margin-top:8px; }
  .chip::after { content:" ×"; color:var(--dim); }
  #cloud .empty {
    position:absolute; inset:0; display:grid; place-items:center; color:var(--dim);
    font-weight:400; transform:none;
  }
  h2 { font-size:13px; font-weight:500; color:var(--dim); margin:20px 0 8px; }
  ol { list-style:none; margin:0; padding:0; columns:2 220px; column-gap:24px; }
  li { display:flex; gap:8px; break-inside:avoid; padding:2px 0; cursor:pointer; }
  li .w { flex:1; overflow:hidden; text-overflow:ellipsis; }
  li .n { color:var(--dim); font-variant-numeric:tabular-nums; }
</style>
</head>
<body>
<main>
  <header><h1>Words</h1><a id="back" href="/">← workbench</a></header>
  <div class="filters">
    <div class="seg" id="speaker" role="group" aria-label="Who">
      <button data-v="" aria-pressed="true">Everyone</button>
      <button data-v="you" aria-pressed="false">You</button>
      <button data-v="agent" aria-pressed="false">Agents</button>
    </div>
    <div class="seg" id="length" role="group" aria-label="Word length">
      <button data-v="0" aria-pressed="true">Any length</button>
      <button data-v="6" aria-pressed="false">6+</button>
      <button data-v="9" aria-pressed="false">9+</button>
    </div>
    <select id="project" aria-label="Project"></select>
    <select id="provider" aria-label="Provider"></select>
    <select id="model" aria-label="Model"></select>
  </div>
  <p class="status" id="status" role="status" aria-live="polite"></p>
  <div id="cloud"></div>
  <div class="actions">
    <span class="hint">Tap a word to hide it and see the words under it.</span>
    <button id="hideAll">Hide all shown</button>
    <button id="undo" disabled>Undo</button>
    <button id="showHidden" aria-expanded="false">Hidden (0)</button>
  </div>
  <div id="hiddenBox" hidden>
    <div class="actions">
      <span class="hint">Tap a word to show it again.</span>
      <button id="unhideAll">Show all again</button>
    </div>
    <div class="chips" id="hiddenList"></div>
  </div>
  <h2>Top words</h2>
  <ol id="list"></ol>
</main>
<script>
const token = new URLSearchParams(location.search).get("t") || "";
const q = p => p + (p.includes("?") ? "&" : "?") + "t=" + encodeURIComponent(token);
document.getElementById("back").href = q("/");
try {
  const theme = localStorage.getItem("theme");
  if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
} catch {}

const state = { speaker: "", project: "", provider: "", model: "", min_len: "" };
let last = null, timer = null, refill = null, shown = [], hidden = [];
const undo = [];
const $ = id => document.getElementById(id);
const fmt = n => n.toLocaleString();
const esc = s => s.replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

for (const b of $("speaker").querySelectorAll("button")) {
  b.onclick = () => {
    state.speaker = b.dataset.v;
    for (const o of $("speaker").querySelectorAll("button")) o.setAttribute("aria-pressed", o === b);
    load();
  };
}
for (const b of $("length").querySelectorAll("button")) {
  b.onclick = () => {
    state.min_len = b.dataset.v === "0" ? "" : b.dataset.v;
    for (const o of $("length").querySelectorAll("button")) o.setAttribute("aria-pressed", o === b);
    load();
  };
}

async function setHidden(words, hide) {
  const res = await fetch(q("/api/words/" + (hide ? "hide" : "unhide")), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ words }),
  });
  if (!res.ok) throw new Error(await res.text());
}

// Hiding is instant on screen; the cloud refills once you pause, so several
// quick taps cost one reload and one new layout, not one each.
async function hide(words) {
  for (const span of $("cloud").querySelectorAll("span[data-w]")) {
    if (words.includes(span.dataset.w)) span.classList.add("gone");
  }
  for (const li of $("list").children) {
    if (words.includes(li.dataset.w)) li.remove();
  }
  undo.push(words);
  $("undo").disabled = false;
  try { await setHidden(words, true); } catch (err) {
    $("status").textContent = "Could not hide: " + err.message;
  }
  clearTimeout(refill);
  refill = setTimeout(load, 1200);
}

$("cloud").onclick = e => {
  const w = e.target.dataset && e.target.dataset.w;
  if (w) hide([w]);
};
$("list").onclick = e => {
  const li = e.target.closest("li");
  if (li) hide([li.dataset.w]);
};
$("hideAll").onclick = () => { if (shown.length) hide(shown.slice()); };
$("undo").onclick = async () => {
  const words = undo.pop();
  $("undo").disabled = !undo.length;
  if (words) { await setHidden(words, false); load(); }
};
$("showHidden").onclick = () => {
  const box = $("hiddenBox");
  box.hidden = !box.hidden;
  $("showHidden").setAttribute("aria-expanded", !box.hidden);
};
$("hiddenList").onclick = async e => {
  const w = e.target.dataset && e.target.dataset.w;
  if (w) { await setHidden([w], false); load(); }
};
$("unhideAll").onclick = async () => {
  if (!hidden.length) return;
  await setHidden(hidden.slice(), false);
  load();
};

for (const id of ["project", "provider", "model"]) {
  $(id).onchange = () => {
    state[id] = $(id).value;
    if (id === "provider") state.model = "";
    load();
  };
}

function fill(select, label, options, value) {
  select.innerHTML = `<option value="">${label}</option>` + options
    .map(([v, n]) => `<option value="${esc(v)}"${v === value ? " selected" : ""}>${esc(v)} · ${fmt(n)}</option>`)
    .join("");
}

async function load() {
  clearTimeout(timer);
  const params = new URLSearchParams({ limit: "150" });
  for (const [k, v] of Object.entries(state)) if (v) params.set(k, v);
  let data;
  try {
    const res = await fetch(q("/api/words?" + params));
    if (!res.ok) throw new Error(await res.text());
    data = await res.json();
  } catch (err) {
    $("status").textContent = "Could not load the counts: " + err.message;
    return;
  }
  const c = data.cloud;
  fill($("project"), "All projects", c.projects, state.project);
  fill($("provider"), "All providers", c.providers, state.provider);
  fill($("model"), "All models", c.models
    .filter(m => !state.provider || m.provider === state.provider)
    .map(m => [m.model, m.words]), state.model);

  const p = data.progress;
  let status = `${fmt(c.total)} words counted, ${fmt(c.words.length)} shown.`;
  if (p.scanning) {
    const pct = p.of ? Math.floor(100 * p.done / p.of) : 0;
    status += ` Reading transcripts: ${pct}%.`;
    timer = setTimeout(load, 3000);
  }
  $("status").textContent = status;

  hidden = c.hidden;
  $("showHidden").textContent = `Hidden (${fmt(hidden.length)})`;
  $("hiddenList").innerHTML = hidden
    .map(w => `<button class="chip" data-w="${esc(w)}">${esc(w)}</button>`).join("");

  const key = JSON.stringify(c.words);
  if (key !== last) {
    last = key;
    draw(c.words);
    $("list").innerHTML = c.words.slice(0, 60)
      .map(([w, n]) => `<li data-w="${esc(w)}" title="Tap to hide"><span class="w">${esc(w)}</span><span class="n">${fmt(n)}</span></li>`)
      .join("");
  }
}

function draw(words) {
  const box = $("cloud");
  box.innerHTML = "";
  shown = [];
  if (!words.length) {
    box.innerHTML = '<div class="empty">No words here yet.</div>';
    return;
  }
  const W = box.clientWidth, H = box.clientHeight;
  const big = Math.max(24, Math.min(W, H) / 8.5), small = 10;
  const measure = document.createElement("canvas").getContext("2d");
  const colors = ["--c1", "--c2", "--c3", "--c4", "--c5"];
  const placed = [];
  const hits = r => placed.some(o =>
    r.x < o.x + o.w && o.x < r.x + r.w && r.y < o.y + o.h && o.y < r.y + r.h);
  const frag = document.createDocumentFragment();

  words.forEach(([word, n], i) => {
    // By rank, not by count: once the common words are hidden the counts
    // left are close together, and sizing by them made every word big and
    // only a dozen fit. Rank keeps a few large words and many small ones.
    const t = words.length > 1 ? Math.pow(1 - i / (words.length - 1), 2.4) : 1;
    const size = Math.round(small + (big - small) * t);
    measure.font = `500 ${size}px "IBM Plex Mono", monospace`;
    const w = measure.measureText(word).width + 6, h = size * 1.05;
    // An Archimedean spiral, stretched to the box's aspect.
    for (let step = 0; step < 2400; step++) {
      const a = step * 0.21, r = 2.2 * a;
      const cx = W / 2 + r * Math.cos(a) * (W / H), cy = H / 2 + r * Math.sin(a);
      const rect = { x: cx - w / 2, y: cy - h / 2, w, h };
      if (rect.x < 4 || rect.y < 4 || rect.x + w > W - 4 || rect.y + h > H - 4) continue;
      if (hits(rect)) continue;
      placed.push(rect);
      const span = document.createElement("span");
      span.textContent = word;
      span.dataset.w = word;
      shown.push(word);
      span.title = `${word}: ${fmt(n)}. Tap to hide.`;
      span.style.cssText = `left:${cx}px;top:${cy}px;font-size:${size}px;` +
        `color:var(${colors[i % colors.length]});opacity:${(0.6 + 0.4 * t).toFixed(2)}`;
      frag.appendChild(span);
      return;
    }
  });
  box.appendChild(frag);
}

let resized;
addEventListener("resize", () => {
  clearTimeout(resized);
  resized = setTimeout(() => { if (last) draw(JSON.parse(last)); }, 200);
});
document.fonts.ready.then(load);
</script>
</body>
</html>
"##;
