//! The repository-map document. Kept in the binary so the feature remains a
//! single executable with no package manager, CDN, or dev server at runtime.

pub const HTML: &str = r###"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light">
<meta name="theme-color" content="#f7f6f1">
<title>Repository map · Workbench</title>
<style>
  :root {
    color-scheme:light;
    --bg:oklch(0.972 0.009 95); --panel:oklch(0.995 0.004 95); --raised:oklch(0.984 0.008 95); --soft:oklch(0.943 0.014 95);
    --ink:oklch(0.25 0.025 255); --dim:oklch(0.47 0.025 255); --faint:oklch(0.61 0.02 255); --line:oklch(0.84 0.02 255);
    --accent:oklch(0.67 0.19 145); --accent-soft:oklch(0.925 0.055 145); --accent-ink:oklch(0.23 0.055 145);
    --blue:oklch(0.55 0.18 258); --amber:oklch(0.69 0.16 76); --danger:oklch(0.58 0.20 27); --violet:oklch(0.58 0.17 305);
    --teal:oklch(0.58 0.13 185); --shadow:oklch(0.28 0.025 255 / .13); --node-w:176px; --node-h:40px;
    --branch:var(--blue); --branch-soft:oklch(0.93 0.035 258); --branch-ink:oklch(0.36 0.15 258);
    font-family:Inter,ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;
  }
  * { box-sizing:border-box }
  [hidden] { display:none!important }
  html,body { width:100%; height:100%; margin:0; overflow:hidden; background:var(--bg); color:var(--ink) }
  button,input,select { font:inherit }
  button { color:inherit }
  button:focus-visible,input:focus-visible,select:focus-visible { outline:2px solid var(--accent); outline-offset:2px }

  .shell { height:100%; display:grid; grid-template-rows:64px minmax(0,1fr) 30px }
  header {
    position:relative; z-index:20; display:flex; align-items:center; gap:16px; padding:10px 16px;
    background:color-mix(in srgb,var(--panel) 94%,transparent);
    box-shadow:0 1px 0 oklch(0.84 0.02 255 / .7),0 10px 30px var(--shadow); backdrop-filter:blur(18px);
  }
  .brand { min-width:max-content; display:flex; align-items:center; gap:11px; margin-right:4px }
  .mark { width:34px; height:34px; display:grid; place-items:center; border-radius:11px; background:var(--accent); color:var(--accent-ink); font:800 16px/1 ui-monospace,monospace; box-shadow:0 0 0 1px oklch(0.30 0.08 145 / .22) inset,0 8px 22px oklch(0.67 0.19 145 / .18) }
  .brand strong { display:block; font-size:13px; letter-spacing:.02em }
  .brand span { display:block; margin-top:2px; color:var(--faint); font:500 10px/1 ui-monospace,monospace; text-transform:uppercase; letter-spacing:.13em }
  .controls { min-width:0; flex:1; display:flex; align-items:center; gap:8px }
  .field,.tool {
    height:40px; border:0; border-radius:10px; color:var(--ink); background:var(--raised);
    box-shadow:0 0 0 1px var(--line) inset,0 1px 2px oklch(0.28 0.025 255 / .08);
  }
  .field { padding:0 12px }
  select.field { width:min(250px,23vw); cursor:pointer }
  .search-wrap { position:relative; flex:1; min-width:150px; max-width:440px }
  .search-wrap::before { content:"⌕"; position:absolute; left:12px; top:8px; color:var(--faint); font-size:19px; pointer-events:none }
  #search { width:100%; padding-left:38px; padding-right:72px }
  #searchCount { position:absolute; right:11px; top:13px; color:var(--faint); font:500 10px/1 ui-monospace,monospace }
  .tool { min-width:40px; padding:0 12px; display:grid; place-items:center; cursor:pointer; transition:background-color 140ms ease,transform 140ms ease,color 140ms ease }
  .tool:hover { background:var(--soft); color:var(--accent) }
  .tool:active { transform:scale(.96) }
  .tool.text { display:flex; gap:7px; white-space:nowrap; color:var(--dim); font-size:12px }
  .tool.text b { color:var(--ink); font-size:15px }
  .divider { width:1px; height:24px; background:var(--line); margin:0 2px }

  #viewport { position:relative; min-width:0; min-height:0; overflow:hidden; cursor:grab; touch-action:none; background:var(--bg) }
  #viewport.panning { cursor:grabbing }
  #world { position:absolute; left:0; top:0; width:1px; height:1px; transform-origin:0 0; will-change:transform }
  #clusters { position:absolute; left:0; top:0; pointer-events:none }
  .cluster { position:absolute; border-radius:14px; background:color-mix(in oklch,var(--branch-soft) 68%,var(--panel)); box-shadow:0 0 0 1px color-mix(in oklch,var(--branch) 28%,var(--line)) inset,0 3px 0 color-mix(in oklch,var(--branch) 62%,transparent) inset,0 10px 28px oklch(0.28 0.025 255 / .07) }
  .cluster::before { content:attr(data-label); position:absolute; left:10px; top:-20px; max-width:calc(100% - 20px); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; padding:4px 7px; border-radius:6px; color:var(--branch-ink); background:color-mix(in oklch,var(--branch-soft) 88%,var(--panel)); box-shadow:0 0 0 1px color-mix(in oklch,var(--branch) 22%,transparent); font:700 8px/1 ui-monospace,monospace; letter-spacing:.08em; text-transform:uppercase }
  #edges { position:absolute; left:0; top:0; overflow:visible; pointer-events:none }
  .edge { fill:none; stroke:var(--branch,oklch(0.66 0.025 255)); stroke-width:1.15; vector-effect:non-scaling-stroke; opacity:.38 }
  .edge.active { stroke:var(--accent); opacity:.9 }
  #nodes { position:absolute; left:0; top:0 }
  .node {
    position:absolute; width:var(--node-w); min-height:var(--node-h); padding:0; display:grid; grid-template-columns:30px minmax(0,1fr) 20px; align-items:center;
    border:0; border-radius:9px; text-align:left; color:var(--ink); background:color-mix(in oklch,var(--branch-soft) 22%,var(--panel));
    box-shadow:0 0 0 1px color-mix(in oklch,var(--branch) 20%,var(--line)) inset,0 1px 2px oklch(0.28 0.025 255 / .10),0 5px 14px oklch(0.28 0.025 255 / .06);
    cursor:pointer; transform:translateZ(0); transition:background-color 140ms ease,box-shadow 140ms ease,scale 140ms ease,opacity 140ms ease;
  }
  .node:not(.root)::before { content:""; position:absolute; left:0; top:7px; bottom:7px; width:3px; border-radius:0 3px 3px 0; background:var(--branch); opacity:.72 }
  .node.directory { background:color-mix(in oklch,var(--branch-soft) 58%,var(--panel)) }
  .node:hover { background:oklch(1 0 0); box-shadow:0 0 0 1px oklch(0.72 0.035 255) inset,0 2px 4px oklch(0.28 0.025 255 / .12),0 12px 30px var(--shadow); z-index:2 }
  .node:active { scale:.96 }
  .node.selected { box-shadow:0 0 0 2px var(--accent),0 1px 2px oklch(0.28 0.025 255 / .10),0 12px 32px var(--shadow) }
  .node.multi-selected { background:oklch(0.94 0.04 258); box-shadow:0 0 0 2px var(--blue),0 2px 4px oklch(0.28 0.025 255 / .12),0 12px 30px oklch(0.55 0.18 258 / .14); z-index:3 }
  .node.agent-highlight { background:color-mix(in oklch,var(--overlay-color,var(--accent)) 13%,var(--panel)); box-shadow:0 0 0 2px color-mix(in oklch,var(--overlay-color,var(--accent)) 78%,transparent),0 2px 5px oklch(0.28 0.025 255 / .12),0 14px 34px color-mix(in oklch,var(--overlay-color,var(--accent)) 16%,transparent); z-index:2 }
  .node.multi-selected.agent-highlight { box-shadow:0 0 0 2px var(--blue),0 0 0 5px color-mix(in oklch,var(--overlay-color,var(--accent)) 45%,transparent),0 14px 34px color-mix(in oklch,var(--overlay-color,var(--accent)) 16%,transparent) }
  .node.match { background:var(--accent-soft); box-shadow:0 0 0 1px oklch(0.67 0.19 145 / .45) inset,0 8px 28px oklch(0.28 0.025 255 / .10) }
  .node.root { background:var(--accent); color:var(--accent-ink); box-shadow:0 0 0 1px oklch(0.30 0.08 145 / .25) inset,0 12px 34px oklch(0.67 0.19 145 / .18) }
  .node-icon { width:22px; height:22px; margin-left:6px; border-radius:6px; display:grid; place-items:center; background:color-mix(in oklch,var(--branch-soft) 86%,var(--panel)); color:var(--branch-ink); box-shadow:0 0 0 1px color-mix(in oklch,var(--branch) 16%,transparent) inset; font:700 9px/1 ui-monospace,monospace }
  .root .node-icon { background:oklch(0.23 0.055 145 / .10); color:inherit }
  .directory .node-icon { color:var(--branch-ink); background:color-mix(in oklch,var(--branch-soft) 95%,var(--panel)) }
  .symlink .node-icon { color:var(--violet) }
  .node-copy { min-width:0; padding:5px 4px 4px 1px }
  .node-name { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font:600 10px/1.15 ui-monospace,SFMono-Regular,Menlo,monospace }
  .node-meta { display:block; margin-top:3px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--faint); font:500 7px/1 ui-monospace,monospace; letter-spacing:.04em }
  .root .node-meta { color:oklch(0.31 0.06 145 / .78) }
  .twisty { width:20px; height:36px; display:grid; place-items:center; color:var(--faint); font-size:12px; transition:rotate 140ms ease,color 140ms ease }
  .node:hover .twisty { color:var(--ink) }
  .directory.open .twisty { rotate:90deg }
  .status-dot { position:absolute; top:6px; right:6px; width:5px; height:5px; border-radius:99px; background:var(--amber); box-shadow:0 0 0 2px var(--panel) }
  .status-dot.untracked { background:var(--accent) }
  .status-dot.added { background:var(--blue) }
  .status-dot.deleted { background:var(--danger) }

  #agentGroups,#agentNotes,#agentDiagrams,#conversationNotes { position:absolute; left:0; top:0; pointer-events:none }
  #agentEdges { position:absolute; left:0; top:0; overflow:visible; pointer-events:none }
  .agent-group { position:absolute; border:2px dashed var(--overlay-color,var(--accent)); border-radius:14px; background:color-mix(in oklch,var(--overlay-color,var(--accent)) 7%,transparent); box-shadow:0 0 0 5px color-mix(in oklch,var(--overlay-color,var(--accent)) 4%,transparent) }
  .agent-group span { position:absolute; left:8px; top:-20px; max-width:240px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; padding:4px 7px; border-radius:6px; color:var(--ink); background:color-mix(in oklch,var(--overlay-color,var(--accent)) 16%,var(--panel)); box-shadow:0 0 0 1px color-mix(in oklch,var(--overlay-color,var(--accent)) 40%,transparent); font:650 8px/1 ui-monospace,monospace }
  .agent-connector { fill:none; stroke:var(--overlay-color,var(--accent)); stroke-width:2; vector-effect:non-scaling-stroke; opacity:.82 }
  .agent-edge-label { fill:var(--dim); font:650 8px ui-monospace,monospace; paint-order:stroke; stroke:var(--bg); stroke-width:4px; stroke-linejoin:round }
  .canvas-callout,.agent-diagram { position:absolute; width:252px; border-radius:14px; color:var(--ink); background:color-mix(in oklch,var(--overlay-color,var(--accent)) 8%,var(--panel)); box-shadow:0 0 0 1px color-mix(in oklch,var(--overlay-color,var(--accent)) 35%,var(--line)) inset,0 2px 6px oklch(0.28 0.025 255 / .10),0 18px 42px var(--shadow); pointer-events:auto }
  .canvas-callout { padding:13px }
  .canvas-callout strong,.agent-diagram > strong { display:block; margin-bottom:7px; font:700 10px/1.25 ui-monospace,monospace }
  .canvas-callout p { margin:0; color:var(--dim); white-space:pre-wrap; font:500 9px/1.5 ui-monospace,monospace }
  .agent-diagram { width:330px; padding:13px }
  .diagram-nodes { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:7px }
  .diagram-node { min-height:38px; display:grid; align-content:center; padding:7px 9px; border-radius:8px; background:var(--panel); box-shadow:0 0 0 1px var(--line) inset; font:600 8.5px/1.3 ui-monospace,monospace }
  .diagram-node[data-path] { color:var(--blue) }
  .diagram-edges { margin-top:9px; display:grid; gap:4px; color:var(--dim); font:500 7.5px/1.3 ui-monospace,monospace }
  .agent-note { position:absolute; width:360px; max-height:520px; display:flex; flex-direction:column; overflow:hidden; border-radius:18px; color:var(--ink); background:color-mix(in oklch,var(--panel) 96%,var(--accent-soft)); box-shadow:0 0 0 1px oklch(0.28 0.025 255 / .08),0 2px 7px oklch(0.28 0.025 255 / .12),0 24px 58px oklch(0.28 0.025 255 / .16); pointer-events:auto }
  .agent-note-head { min-height:58px; display:flex; align-items:center; gap:10px; padding:9px 9px 9px 13px; cursor:grab; user-select:none; box-shadow:0 1px 0 var(--line) }
  .agent-note-head:active { cursor:grabbing }
  .agent-note-mark { flex:none; width:34px; height:34px; display:grid; place-items:center; border-radius:10px; color:var(--accent-ink); background:var(--accent-soft); font:750 13px/1 ui-monospace,monospace }
  .agent-note-title { min-width:0; flex:1 }
  .agent-note-title strong { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:11px }
  .agent-note-title span { display:block; margin-top:4px; color:var(--faint); font:600 8px/1 ui-monospace,monospace }
  .agent-note-close { flex:none; width:40px; height:40px; min-width:40px; font-size:18px }
  .agent-note-context { padding:8px 13px; color:var(--faint); background:var(--raised); box-shadow:0 1px 0 var(--line); font:600 8px/1.3 ui-monospace,monospace }
  .agent-thread { min-height:86px; max-height:288px; display:grid; gap:12px; overflow:auto; padding:13px; scrollbar-width:thin; scrollbar-color:oklch(0.73 0.02 255) transparent }
  .agent-note-empty { align-self:center; padding:14px 12px; text-align:center; color:var(--faint); font:500 9px/1.55 ui-monospace,monospace }
  .agent-turn { display:grid; gap:7px }
  .agent-question { justify-self:end; max-width:88%; padding:8px 10px; border-radius:12px 12px 4px 12px; color:var(--accent-ink); background:var(--accent-soft); white-space:pre-wrap; overflow-wrap:anywhere; font:600 9px/1.45 ui-monospace,monospace }
  .agent-answer { margin:0; padding:10px 11px; border-radius:12px 12px 12px 4px; color:var(--dim); background:var(--raised); box-shadow:0 0 0 1px oklch(0.28 0.025 255 / .07); white-space:pre-wrap; overflow-wrap:anywhere; font:500 9.5px/1.58 ui-monospace,monospace }
  .agent-answer.error { color:var(--danger); background:color-mix(in oklch,var(--danger) 7%,var(--raised)) }
  .agent-note-status { min-height:22px; display:flex; align-items:center; gap:7px; padding:0 13px; color:var(--faint); font:650 8px/1 ui-monospace,monospace; text-transform:uppercase; letter-spacing:.07em }
  .agent-note-status::before { content:""; width:7px; height:7px; border-radius:50%; background:var(--accent) }
  .agent-note-status.working::before { animation:pulse 1.2s ease-in-out infinite }
  .agent-note-status.error::before { background:var(--danger) }
  .agent-note-compose { display:grid; grid-template-columns:minmax(0,1fr) 44px; gap:8px; padding:8px; border-radius:18px; background:var(--panel); box-shadow:0 -1px 0 var(--line) }
  .agent-note-prompt { min-height:44px; max-height:112px; resize:vertical; padding:9px 10px; border:0; border-radius:10px; color:var(--ink); background:var(--raised); box-shadow:0 0 0 1px var(--line) inset; font:500 9.5px/1.45 ui-monospace,monospace }
  .agent-note-send { width:44px; height:44px; padding:0; color:var(--accent-ink); background:var(--accent); box-shadow:0 0 0 1px oklch(0.30 0.08 145 / .22) inset,0 8px 20px oklch(0.67 0.19 145 / .16) }
  .agent-note-send:hover { color:var(--accent-ink); background:oklch(0.71 0.18 145) }
  .agent-note-send:disabled { cursor:not-allowed; opacity:.45; transform:none }
  #world.architecture-mode #clusters,#world.architecture-mode #edges,#world.architecture-mode #nodes,#world.architecture-mode #agentGroups,#world.architecture-mode #agentEdges,#world.architecture-mode #agentNotes,#world.architecture-mode #agentDiagrams { display:none }
  #architectureView { display:none; position:absolute; left:0; top:0 }
  #world.architecture-mode #architectureView { display:block }
  #architectureEdges { position:absolute; left:0; top:0; overflow:visible; pointer-events:none }
  #architectureNodes { position:absolute; left:0; top:0 }
  .architecture-edge { fill:none; stroke:color-mix(in oklch,var(--map-color,var(--blue)) 58%,var(--line)); stroke-width:2; vector-effect:non-scaling-stroke; opacity:.72 }
  .architecture-edge-label { fill:var(--dim); font:650 9px ui-monospace,monospace; paint-order:stroke; stroke:var(--bg); stroke-width:5px; stroke-linejoin:round }
  .architecture-card { position:absolute; width:284px; height:154px; display:flex; flex-direction:column; gap:9px; padding:15px; border:0; border-radius:15px; overflow:hidden; text-align:left; color:var(--ink); background:color-mix(in oklch,var(--map-color,var(--blue)) 7%,var(--panel)); box-shadow:0 0 0 1px color-mix(in oklch,var(--map-color,var(--blue)) 34%,var(--line)) inset,0 3px 0 color-mix(in oklch,var(--map-color,var(--blue)) 72%,transparent) inset,0 2px 5px oklch(0.28 0.025 255 / .10),0 18px 42px oklch(0.28 0.025 255 / .10); cursor:pointer; transition:background-color 140ms ease,box-shadow 140ms ease,scale 140ms ease }
  .architecture-card:hover { background:color-mix(in oklch,var(--map-color,var(--blue)) 12%,var(--panel)); box-shadow:0 0 0 2px color-mix(in oklch,var(--map-color,var(--blue)) 62%,var(--line)) inset,0 3px 0 var(--map-color,var(--blue)) inset,0 3px 7px oklch(0.28 0.025 255 / .11),0 22px 50px oklch(0.28 0.025 255 / .14) }
  .architecture-card:active { scale:.96 }
  .architecture-card.selected { background:color-mix(in oklch,var(--map-color,var(--blue)) 15%,var(--panel)); box-shadow:0 0 0 3px var(--map-color,var(--blue)) inset,0 2px 5px oklch(0.28 0.025 255 / .10),0 22px 52px color-mix(in oklch,var(--map-color,var(--blue)) 18%,transparent) }
  .architecture-eyebrow { display:flex; align-items:center; justify-content:space-between; gap:10px; color:color-mix(in oklch,var(--map-color,var(--blue)) 82%,var(--ink)); font:700 8px/1 ui-monospace,monospace; letter-spacing:.09em; text-transform:uppercase }
  .architecture-card-title { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:15px; font-weight:700; line-height:1.2 }
  .architecture-card-copy { min-height:34px; display:-webkit-box; overflow:hidden; color:var(--dim); font-size:10.5px; line-height:1.55; -webkit-box-orient:vertical; -webkit-line-clamp:2 }
  .architecture-paths { margin-top:auto; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--faint); font:550 8px/1.2 ui-monospace,monospace }
  #architectureNav { position:absolute; z-index:13; left:14px; top:14px; width:min(880px,calc(100% - 28px)); min-height:58px; display:grid; grid-template-columns:auto minmax(160px,1fr) auto auto auto; align-items:center; gap:8px; padding:9px; border-radius:16px; background:oklch(0.995 0.004 95 / .96); box-shadow:0 0 0 1px var(--line) inset,0 2px 7px oklch(0.28 0.025 255 / .10),0 18px 44px var(--shadow); backdrop-filter:blur(18px) }
  .architecture-crumbs { min-width:0; padding:0 6px }
  .architecture-crumbs strong { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:11px }
  .architecture-crumbs span { display:block; margin-top:4px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--faint); font:550 8px/1 ui-monospace,monospace }
  #architectureBack { min-width:44px }
  #categorizeJob[aria-pressed="true"] { color:oklch(0.36 0.15 258); background:oklch(0.93 0.035 258); box-shadow:0 0 0 2px oklch(0.55 0.18 258 / .38) inset,0 1px 2px oklch(0.28 0.025 255 / .08) }
  #lassoBox { position:absolute; z-index:12; border:1.5px solid var(--blue); border-radius:6px; background:oklch(0.55 0.18 258 / .10); box-shadow:0 0 0 1px oklch(1 0 0 / .6) inset; pointer-events:none }
  #viewport.selecting { cursor:crosshair }
  #selectTool[aria-pressed="true"] { color:var(--blue); background:oklch(0.92 0.04 258); box-shadow:0 0 0 2px oklch(0.55 0.18 258 / .45) inset,0 1px 2px oklch(0.28 0.025 255 / .08) }

  .empty { position:absolute; z-index:4; inset:0; display:grid; place-items:center; pointer-events:none }
  .empty-card { width:min(390px,calc(100% - 40px)); padding:28px; border-radius:18px; text-align:center; background:oklch(0.995 0.004 95 / .92); box-shadow:0 0 0 1px var(--line) inset,0 20px 60px var(--shadow); backdrop-filter:blur(14px) }
  .empty-card .glyph { width:48px; height:48px; margin:0 auto 16px; display:grid; place-items:center; border-radius:15px; color:var(--accent-ink); background:var(--accent-soft); font:700 20px/1 ui-monospace,monospace }
  .empty-card h2 { margin:0 0 8px; font-size:15px }
  .empty-card p { margin:0; color:var(--dim); font-size:12px; line-height:1.6 }
  .spinner { animation:spin 900ms linear infinite }
  @keyframes spin { to { rotate:360deg } }

  #inspector { position:absolute; z-index:8; right:14px; top:14px; width:300px; padding:16px; border-radius:16px; background:oklch(0.995 0.004 95 / .94); box-shadow:0 0 0 1px var(--line) inset,0 20px 50px var(--shadow); backdrop-filter:blur(16px); transform:translateX(calc(100% + 28px)); opacity:0; transition:transform 180ms cubic-bezier(.2,0,0,1),opacity 150ms ease }
  #inspector.open { transform:translateX(0); opacity:1 }
  .inspector-top { display:flex; align-items:start; gap:10px }
  .inspector-kind { flex:none; width:34px; height:34px; display:grid; place-items:center; border-radius:10px; color:var(--blue); background:oklch(0.92 0.04 258); font:700 13px/1 ui-monospace,monospace }
  .inspector-title { min-width:0; flex:1 }
  .inspector-title strong { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font:600 12px/1.35 ui-monospace,monospace }
  .inspector-title span { display:block; margin-top:4px; color:var(--faint); font-size:10px }
  #closeInspector { flex:none; width:32px; height:32px; min-width:32px }
  .detail { margin-top:16px; display:grid; grid-template-columns:72px minmax(0,1fr); gap:9px 12px; font:500 10px/1.4 ui-monospace,monospace }
  .detail dt { color:var(--faint) }
  .detail dd { margin:0; color:var(--dim); overflow-wrap:anywhere }
  .copy-path { width:100%; margin-top:14px }

  #minimap { position:absolute; z-index:6; left:14px; bottom:14px; width:184px; height:112px; border-radius:13px; touch-action:none; cursor:crosshair; background:oklch(0.995 0.004 95 / .9); box-shadow:0 0 0 1px var(--line) inset,0 10px 30px oklch(0.28 0.025 255 / .11); backdrop-filter:blur(12px); transition:background-color 140ms ease,box-shadow 140ms ease }
  #minimap:hover,#minimap:focus-visible { background:var(--panel); box-shadow:0 0 0 2px oklch(0.55 0.18 258 / .38),0 12px 34px oklch(0.28 0.025 255 / .16) }
  #minimap.navigating { cursor:grabbing; background:var(--panel); box-shadow:0 0 0 2px var(--blue),0 14px 38px oklch(0.28 0.025 255 / .19) }
  #layerTools { position:absolute; z-index:7; left:212px; bottom:14px; display:flex; gap:8px; padding:6px; border-radius:16px; background:oklch(0.995 0.004 95 / .92); box-shadow:0 0 0 1px oklch(0.28 0.025 255 / .07),0 10px 30px oklch(0.28 0.025 255 / .11); backdrop-filter:blur(12px) }
  #layerTools .tool { min-height:40px }
  .hint { position:absolute; z-index:5; right:14px; bottom:15px; padding:7px 10px; border-radius:9px; color:var(--faint); background:oklch(0.995 0.004 95 / .88); box-shadow:0 0 0 1px var(--line) inset; font:500 9px/1 ui-monospace,monospace; pointer-events:none }
  footer { z-index:15; display:flex; align-items:center; gap:18px; padding:0 16px; color:var(--faint); background:var(--panel); box-shadow:0 -1px 0 var(--line); font:500 9px/1 ui-monospace,monospace; letter-spacing:.02em }
  footer .live { display:flex; align-items:center; gap:7px; color:var(--dim) }
  footer .live::before { content:""; width:6px; height:6px; border-radius:50%; background:var(--accent); box-shadow:0 0 10px oklch(0.67 0.19 145 / .55) }
  #summary { margin-left:auto }
  kbd { padding:2px 5px; border-radius:5px; color:var(--dim); background:var(--soft); box-shadow:0 0 0 1px var(--line) inset; font:500 9px/1 ui-monospace,monospace }

  @keyframes pulse { 50% { opacity:.35; scale:.76 } }
  .file-viewer {
    position:fixed; z-index:60; inset:0; display:grid; place-items:center; padding:24px;
    background:oklch(0.30 0.025 255 / .20); backdrop-filter:blur(9px); opacity:0; pointer-events:none;
    transition:opacity 160ms ease;
  }
  .file-viewer.open { opacity:1; pointer-events:auto }
  .code-dialog {
    width:min(1120px,calc(100vw - 48px)); height:min(760px,calc(100vh - 48px)); min-height:300px;
    display:grid; grid-template-rows:auto minmax(0,1fr); border-radius:20px; overflow:hidden; background:var(--panel);
    box-shadow:0 1px 2px oklch(0.28 0.025 255 / .12),0 18px 48px oklch(0.28 0.025 255 / .20),0 48px 110px oklch(0.28 0.025 255 / .16);
    transform:translateY(12px) scale(.985); transition:transform 180ms cubic-bezier(.2,0,0,1);
  }
  .file-viewer.open .code-dialog { transform:translateY(0) scale(1) }
  .code-head { min-width:0; display:flex; align-items:center; gap:12px; padding:12px }
  .code-file-icon { flex:none; width:40px; height:40px; display:grid; place-items:center; border-radius:12px; color:var(--blue); background:oklch(0.92 0.04 258); font:750 10px/1 ui-monospace,monospace }
  .code-title-wrap { min-width:0; flex:1 }
  .code-title-row { min-width:0; display:flex; align-items:center; gap:9px }
  #codeTitle { margin:0; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font:650 13px/1.3 ui-monospace,SFMono-Regular,Menlo,monospace }
  #codeLanguage { flex:none; padding:4px 7px; border-radius:6px; color:var(--blue); background:oklch(0.92 0.04 258); font:650 9px/1 ui-monospace,monospace }
  #codePath { display:block; margin-top:4px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--faint); font:500 9px/1.2 ui-monospace,monospace }
  .code-actions { flex:none; display:flex; gap:8px }
  .code-actions .tool { min-width:40px }
  #copyCode { display:flex; gap:7px; color:var(--dim); font-size:11px }
  #closeCode { font-size:19px }
  .code-surface {
    position:relative; min-width:0; min-height:0; margin:0 12px 12px; overflow:auto; border-radius:12px;
    background:oklch(0.978 0.008 95); box-shadow:0 0 0 1px var(--line) inset;
    scrollbar-color:oklch(0.73 0.02 255) transparent; scrollbar-width:thin;
  }
  #codeLines { min-width:max-content; padding:9px 0 }
  .code-line { min-width:100%; min-height:21px; display:grid; grid-template-columns:58px minmax(max-content,1fr); font:500 11.5px/21px ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace }
  .line-number { position:sticky; left:0; z-index:1; padding-right:14px; text-align:right; user-select:none; color:oklch(0.66 0.02 255); background:oklch(0.947 0.012 95); box-shadow:1px 0 0 var(--line) }
  .line-code { padding:0 20px; color:var(--ink); white-space:pre; tab-size:4 }
  .tok-comment { color:oklch(0.54 0.055 150); font-style:italic }
  .tok-keyword { color:oklch(0.48 0.19 292); font-weight:650 }
  .tok-string { color:oklch(0.46 0.12 145) }
  .tok-number { color:oklch(0.54 0.17 55) }
  .tok-literal { color:oklch(0.50 0.18 258); font-weight:650 }
  .tok-function { color:oklch(0.46 0.16 258) }
  .tok-type { color:oklch(0.46 0.11 205) }
  .tok-property { color:oklch(0.45 0.12 318) }
  .tok-punctuation { color:oklch(0.47 0.025 255) }
  .code-state { position:absolute; inset:0; display:grid; place-items:center; padding:30px; text-align:center; color:var(--dim); background:oklch(0.978 0.008 95); font:500 11px/1.6 ui-monospace,monospace }
  .code-state strong { display:block; margin-bottom:5px; color:var(--ink); font-size:12px }
  .code-meta { position:sticky; left:0; bottom:0; z-index:3; width:max-content; margin:8px 10px 10px auto; padding:5px 8px; border-radius:7px; color:var(--faint); background:oklch(0.995 0.004 95 / .92); box-shadow:0 0 0 1px var(--line); backdrop-filter:blur(8px); font:500 9px/1 ui-monospace,monospace }

  @media (max-width:760px) {
    .shell { grid-template-rows:auto minmax(0,1fr) 30px }
    header { align-items:flex-start; flex-wrap:wrap; padding:10px }
    .brand { width:100% }
    .controls { width:100%; flex-wrap:wrap }
    select.field { width:calc(100% - 49px); flex:1 }
    .search-wrap { order:2; flex-basis:100%; max-width:none }
    .tool.text,.divider { display:none }
    #selectTool,#agentTool,#analyzeJob,#categorizeJob { display:flex; min-width:44px; padding:0 11px }
    #inspector { left:10px; right:10px; top:auto; bottom:10px; width:auto; transform:translateY(calc(100% + 20px)) }
    #inspector.open { transform:translateY(0) }
    #minimap { width:130px; height:82px }
    #layerTools { left:150px; bottom:10px }
    #layerTools .tool { min-width:44px; padding:0 10px }
    #layerTools .tool b { display:none }
    .hint { display:none }
    .agent-note { width:min(360px,calc(100vw - 28px)); max-height:68vh }
    #architectureNav { left:10px; top:10px; width:calc(100% - 20px); grid-template-columns:repeat(3,minmax(0,1fr)); border-radius:15px }
    #architectureBack { grid-column:1; grid-row:1 }
    .architecture-crumbs { grid-column:2 / -1; grid-row:1 }
    #architectureFiles { grid-column:1; grid-row:2 }
    #architectureDrill { grid-column:2; grid-row:2 }
    #architectureRemap { grid-column:3; grid-row:2 }
    #architectureNav .tool.text { display:flex; justify-content:center; min-width:44px; padding:0 8px }
    .file-viewer { padding:10px }
    .code-dialog { width:calc(100vw - 20px); height:calc(100vh - 20px); border-radius:16px }
    .code-head { gap:8px; padding:10px }
    .code-file-icon { display:none }
    #copyCode span { display:none }
    .code-surface { margin:0 10px 10px; border-radius:8px }
    .code-line { grid-template-columns:46px minmax(max-content,1fr); font-size:11px }
    .line-number { padding-right:10px }
    .line-code { padding:0 14px }
  }
  @media (prefers-reduced-motion:reduce) { *,*::before,*::after { scroll-behavior:auto!important; animation-duration:.01ms!important; transition-duration:.01ms!important } }
</style>
</head>
<body>
<div class="shell">
  <header>
    <div class="brand"><div class="mark">W/</div><div><strong>Repository map</strong><span>Workbench canvas</span></div></div>
    <div class="controls">
      <select class="field" id="workspace" aria-label="Workspace"></select>
      <div class="search-wrap"><input class="field" id="search" type="search" placeholder="Find a file or folder…" autocomplete="off"><span id="searchCount"></span></div>
      <button class="tool text" id="refresh" title="Rescan repository"><b>↻</b> Refresh</button>
      <div class="divider"></div>
      <button class="tool text" id="selectTool" type="button" aria-pressed="false" title="Select files and regions"><b>⌁</b> Select</button>
      <button class="tool text" id="analyzeJob" type="button" title="Run the Analyze repository job in a new agent note"><b>◎</b> Analyze</button>
      <button class="tool text" id="categorizeJob" type="button" aria-pressed="false" title="Run the Categorize repository job in a new agent note"><b>◇</b> Categorize</button>
      <button class="tool text" id="agentTool" type="button" title="Create an independent Claude agent note"><b>＋</b> Note</button>
      <button class="tool" id="zoomOut" title="Zoom out" aria-label="Zoom out">−</button>
      <button class="tool" id="zoomIn" title="Zoom in" aria-label="Zoom in">+</button>
      <button class="tool" id="fit" title="Fit visible tree" aria-label="Fit visible tree">⌗</button>
    </div>
  </header>
  <main id="viewport" tabindex="0" aria-label="Interactive repository file tree">
    <div id="world"><div id="clusters"></div><div id="agentGroups"></div><svg id="edges"></svg><svg id="agentEdges"></svg><div id="nodes"></div><div id="agentNotes"></div><div id="agentDiagrams"></div><div id="architectureView"><svg id="architectureEdges"></svg><div id="architectureNodes"></div></div><div id="conversationNotes"></div></div>
    <div id="lassoBox" hidden></div>
    <div class="empty" id="empty"><div class="empty-card"><div class="glyph spinner">◌</div><h2>Mapping repository</h2><p>Reading the file tree and arranging the canvas.</p></div></div>
    <aside id="inspector" aria-live="polite">
      <div class="inspector-top"><div class="inspector-kind" id="inspectIcon">F</div><div class="inspector-title"><strong id="inspectName"></strong><span id="inspectKind"></span></div><button class="tool" id="closeInspector" aria-label="Close details">×</button></div>
      <dl class="detail" id="details"></dl>
      <button class="tool text copy-path" id="copyPath"><b>⧉</b> Copy relative path</button>
    </aside>
    <canvas id="minimap" width="368" height="224" tabindex="0" aria-label="Repository minimap. Click or drag to navigate; use arrow keys to pan." title="Click or drag to navigate the canvas"></canvas>
    <nav id="architectureNav" aria-label="Architecture lens navigation" hidden><button class="tool" id="architectureBack" type="button" title="Previous architecture level" aria-label="Previous architecture level">←</button><div class="architecture-crumbs"><strong id="architectureTitle">Architecture lens</strong><span id="architectureSummary"></span></div><button class="tool text secondary-action" id="architectureFiles" type="button" disabled><b>⌘</b> Files</button><button class="tool text secondary-action" id="architectureDrill" type="button" disabled><b>↳</b> Drill in</button><button class="tool text" id="architectureRemap" type="button"><b>↻</b> Remap</button></nav>
    <div id="layerTools" hidden><button class="tool text" id="undoLayer" type="button"><b>↶</b> Undo AI drawing</button><button class="tool text" id="clearLayers" type="button"><b>×</b> Clear AI drawings</button></div>
    <div class="hint">Click files to preview · Select or <kbd>Shift</kbd>-drag to lasso · <kbd>⌘/Ctrl</kbd> + wheel to zoom</div>
  </main>
  <footer><span class="live">Local &amp; read-only</span><span id="updated">Waiting for repository</span><span>Agent notes are temporary</span><span id="summary"></span></footer>
</div>
<div class="file-viewer" id="fileViewer" hidden>
  <section class="code-dialog" role="dialog" aria-modal="true" aria-labelledby="codeTitle">
    <div class="code-head">
      <div class="code-file-icon" id="codeIcon">F</div>
      <div class="code-title-wrap"><div class="code-title-row"><h2 id="codeTitle">File preview</h2><span id="codeLanguage">Text</span></div><span id="codePath"></span></div>
      <div class="code-actions"><button class="tool" id="copyCode" type="button" title="Copy file contents"><b>⧉</b><span>Copy</span></button><button class="tool" id="closeCode" type="button" aria-label="Close file preview">×</button></div>
    </div>
    <div class="code-surface" id="codeSurface" tabindex="0">
      <div id="codeLines"></div>
      <div class="code-state" id="codeState"><div><strong>Loading file</strong>Reading the latest contents from disk.</div></div>
      <div class="code-meta" id="codeMeta"></div>
    </div>
  </section>
</div>
<script>
(() => {
  "use strict";
  const $ = id => document.getElementById(id);
  const viewport=$('viewport'), world=$('world'), clustersEl=$('clusters'), nodesEl=$('nodes'), edgesEl=$('edges');
  const agentGroups=$('agentGroups'), agentEdges=$('agentEdges'), agentNotes=$('agentNotes'), agentDiagrams=$('agentDiagrams'), conversationNotes=$('conversationNotes');
  const architectureNodes=$('architectureNodes'), architectureEdges=$('architectureEdges');
  const workspaceEl=$('workspace'), searchEl=$('search'), emptyEl=$('empty'), inspector=$('inspector');
  const fileViewer=$('fileViewer'), codeLines=$('codeLines'), codeState=$('codeState');
  const NODE_W=176, NODE_H=40, ARCH_W=284, ARCH_H=154, PAD=64, ROOT_GAP=214, HUB_STEP=46, GRID_X=10, GRID_Y=6, LEVEL_GAP=38, CLUSTER_GAP=58, CLUSTER_COL_GAP=52;
  const state={workspaces:[],data:null,root:null,collapsed:new Set(),selected:null,selection:new Set(),selectMode:false,viewMode:'tree',architectureIndex:-1,architectureSelection:null,architectureLayout:new Map(),layout:new Map(),bounds:{x:0,y:0,w:1,h:1},x:60,y:60,scale:1,query:'',drag:null,lasso:null,minimapDrag:null,noteDrag:null,loadedOnce:false,refreshing:false,fileRequest:0,codeContent:'',lastFocus:null,notes:[],noteSerial:0,layers:[],layerSerial:0};
  const CANVAS_AGENT_LABEL='Claude Code · Sonnet 5';
  const overlayColors={green:'var(--accent)',blue:'var(--blue)',amber:'var(--amber)',violet:'var(--violet)',red:'var(--danger)'};
  const mapCanvasColors={green:'#38a852',blue:'#4b78cb',amber:'#bd862c',violet:'#9a62b6',red:'#c84f42'};
  const branchPalette=[
    {color:'oklch(0.55 0.18 258)',soft:'oklch(0.93 0.035 258)',ink:'oklch(0.36 0.15 258)',canvas:'#4b78cb'},
    {color:'oklch(0.58 0.17 305)',soft:'oklch(0.94 0.035 305)',ink:'oklch(0.38 0.13 305)',canvas:'#9a62b6'},
    {color:'oklch(0.69 0.16 76)',soft:'oklch(0.95 0.045 76)',ink:'oklch(0.40 0.11 76)',canvas:'#bd862c'},
    {color:'oklch(0.58 0.13 185)',soft:'oklch(0.94 0.035 185)',ink:'oklch(0.35 0.10 185)',canvas:'#3a8d8d'}
  ];

  function setBranchStyle(element,branch=0){const palette=branchPalette[branch%branchPalette.length];element.style.setProperty('--branch',palette.color);element.style.setProperty('--branch-soft',palette.soft);element.style.setProperty('--branch-ink',palette.ink)}

  const escapePath = value => encodeURIComponent(value);
  const formatBytes = bytes => {
    if(bytes == null) return '—';
    if(bytes < 1024) return `${bytes} B`;
    const units=['KB','MB','GB']; let value=bytes/1024, i=0;
    while(value>=1024 && i<units.length-1){value/=1024;i++}
    return `${value<10?value.toFixed(1):Math.round(value)} ${units[i]}`;
  };
  const iconFor = node => node.kind==='root'?'W':node.kind==='directory'?'D':node.kind==='symlink'?'↗':(node.extension||'F').slice(0,2).toUpperCase();
  const metaFor = node => node.kind==='directory'?`${node.children.length} item${node.children.length===1?'':'s'}`:node.status||node.extension||node.kind;
  const literals=new Set(['true','false','null','None','True','False','nil','undefined']);
  const keywords={
    rust:new Set('as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while'.split(' ')),
    javascript:new Set('as async await break case catch class const continue debugger default delete do else export extends finally for from function get if implements import in instanceof interface let new of package private protected public return set static super switch this throw try typeof var void while with yield'.split(' ')),
    python:new Set('and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield'.split(' ')),
    go:new Set('break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var'.split(' ')),
    ruby:new Set('alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield'.split(' ')),
    c:new Set('alignas alignof auto bool break case catch char class const constexpr continue default delete do double else enum explicit export extern false float for friend goto if inline int long namespace new nullptr operator private protected public register return short signed sizeof static struct switch template this throw true try typedef typename union unsigned using virtual void volatile wchar_t while'.split(' ')),
    swift:new Set('associatedtype break case catch class continue default defer deinit do else enum extension fallthrough false fileprivate for func guard if import in init inout internal is let nil open operator private protocol public repeat rethrows return self static struct subscript super switch throw throws true try typealias var where while'.split(' ')),
    kotlin:new Set('as break class continue do else false for fun if in interface is null object package return super this throw true try typealias typeof val var when while'.split(' ')),
    shell:new Set('case do done elif else esac fi for function if in select then time until while'.split(' ')),
    sql:new Set('all alter and as asc begin between by case create delete desc distinct drop else end exists from full group having in inner insert into is join left like limit not null on or order outer primary references right select set table then union unique update values when where'.split(' ')),
    css:new Set('@charset @container @font-face @import @keyframes @layer @media @page @supports'.split(' ')),
    generic:new Set('class const def else enum fn for function if impl import let module package private protected public return static struct type use var while'.split(' '))
  };

  function languageKey(label){
    const value=(label||'').toLowerCase();
    if(['javascript','typescript','jsx','tsx','vue','svelte'].includes(value))return 'javascript';
    if(['c','c++','c#','java'].includes(value))return 'c';
    if(['html','xml'].includes(value))return 'markup';
    if(['json','toml','yaml','css','markdown','python','rust','go','ruby','swift','kotlin','shell','sql'].includes(value))return value;
    return 'generic';
  }

  function appendToken(parent,kind,value){
    if(!value)return;
    if(!kind){parent.append(document.createTextNode(value));return}
    const token=document.createElement('span');token.className=`tok-${kind}`;token.textContent=value;parent.append(token);
  }

  function lineComment(language){
    if(['python','ruby','shell','yaml','toml'].includes(language))return '#';
    if(language==='sql')return '--';
    if(['rust','javascript','go','c','swift','kotlin','generic'].includes(language))return '//';
    return null;
  }

  function tokenizeLine(raw,language,syntax){
    const output=document.createDocumentFragment(); let index=0;
    while(index<raw.length){
      if(syntax.block){
        const end=syntax.block==='markup'?'-->':'*/', found=raw.indexOf(end,index);
        if(found<0){appendToken(output,'comment',raw.slice(index));return output}
        appendToken(output,'comment',raw.slice(index,found+end.length));index=found+end.length;syntax.block=null;continue;
      }
      if(language==='markup'&&raw.startsWith('<!--',index)){
        const found=raw.indexOf('-->',index+4);
        if(found<0){syntax.block='markup';appendToken(output,'comment',raw.slice(index));return output}
        appendToken(output,'comment',raw.slice(index,found+3));index=found+3;continue;
      }
      if(raw.startsWith('/*',index)){
        const found=raw.indexOf('*/',index+2);
        if(found<0){syntax.block='slash';appendToken(output,'comment',raw.slice(index));return output}
        appendToken(output,'comment',raw.slice(index,found+2));index=found+2;continue;
      }
      const comment=lineComment(language);
      if(comment&&raw.startsWith(comment,index)){appendToken(output,'comment',raw.slice(index));return output}
      const char=raw[index];
      if(char==='"'||char==="'"||char==='`'){
        let end=index+1,escaped=false;
        while(end<raw.length){const next=raw[end];if(next===char&&!escaped){end++;break}escaped=next==='\\'&&!escaped; if(next!=='\\')escaped=false;end++}
        appendToken(output,'string',raw.slice(index,end));index=end;continue;
      }
      if(/[0-9]/.test(char)){
        let end=index+1;while(end<raw.length&&/[0-9A-Fa-f_xXob.]/.test(raw[end]))end++;
        appendToken(output,'number',raw.slice(index,end));index=end;continue;
      }
      if(/[A-Za-z_$]/.test(char)){
        let end=index+1;while(end<raw.length&&/[A-Za-z0-9_$-]/.test(raw[end]))end++;
        const word=raw.slice(index,end),next=raw.slice(end).match(/^\s*([(:=>])/),previous=raw.slice(0,index).match(/([^\s])\s*$/);
        const set=keywords[language]||keywords.generic; let kind='';
        if(literals.has(word))kind='literal';
        else if(set.has(word))kind='keyword';
        else if(next?.[1]==='(')kind='function';
        else if((['json','toml','yaml','css'].includes(language)&&next)||language==='markup'&&(previous?.[1]==='<'))kind='property';
        else if(/^[A-Z]/.test(word))kind='type';
        appendToken(output,kind,word);index=end;continue;
      }
      if('{}[]()<>,.:;=+-*/!&|?%^~@#'.includes(char)){appendToken(output,'punctuation',char);index++;continue}
      let end=index+1;while(end<raw.length&&!/[A-Za-z0-9_$'"`{}[\]()<>,.:;=+*/!&|?%^~@#-]/.test(raw[end]))end++;
      appendToken(output,'',raw.slice(index,end));index=end;
    }
    return output;
  }

  async function getJson(url){
    const response=await fetch(url,{cache:'no-store'});
    if(!response.ok) throw new Error(await response.text()||`Request failed (${response.status})`);
    return response.json();
  }

  async function postJson(url,body){
    const response=await fetch(url,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});
    if(!response.ok)throw new Error(await response.text()||`Request failed (${response.status})`);
    return response.json();
  }

  function showCodeState(title,message){
    codeLines.hidden=true;$('codeMeta').hidden=true;codeState.hidden=false;codeState.replaceChildren();
    const wrap=document.createElement('div'),heading=document.createElement('strong');heading.textContent=title;wrap.append(heading,document.createTextNode(message));codeState.append(wrap);
  }

  function revealFileViewer(node){
    state.lastFocus=document.activeElement;document.querySelector('.shell').inert=true;fileViewer.hidden=false;fileViewer.setAttribute('aria-busy','true');
    $('codeTitle').textContent=node.name;$('codePath').textContent=node.path;$('codeLanguage').textContent=node.extension?.toUpperCase()||'Text';$('codeIcon').textContent=iconFor(node);
    showCodeState('Loading file','Reading the latest contents from disk.');
    requestAnimationFrame(()=>{fileViewer.classList.add('open');$('closeCode').focus({preventScroll:true})});
  }

  function renderFile(data){
    const normalized=data.content.replace(/\r\n?/g,'\n'),allLines=normalized.split('\n'),limit=12000,shown=allLines.slice(0,limit),syntax={block:null},language=languageKey(data.language);
    const fragment=document.createDocumentFragment();
    shown.forEach((line,index)=>{
      const row=document.createElement('div');row.className='code-line';
      const number=document.createElement('span');number.className='line-number';number.textContent=String(index+1);
      const code=document.createElement('span');code.className='line-code';code.append(tokenizeLine(line,language,syntax));row.append(number,code);fragment.append(row);
    });
    codeLines.replaceChildren(fragment);codeLines.hidden=false;codeState.hidden=true;$('codeMeta').hidden=false;
    const clipped=data.truncated||allLines.length>limit;
    $('codeMeta').textContent=`${shown.length.toLocaleString()} line${shown.length===1?'':'s'} · ${formatBytes(data.bytes)}${clipped?' · preview truncated':''}`;
    $('codeTitle').textContent=data.name;$('codePath').textContent=data.path;$('codeLanguage').textContent=data.language;$('codeIcon').textContent=(data.extension||'F').slice(0,2).toUpperCase();
    $('codeSurface').scrollTo(0,0);fileViewer.setAttribute('aria-busy','false');state.codeContent=data.content;
  }

  async function openFile(node){
    selectNode(node);revealFileViewer(node);const request=++state.fileRequest;
    try{
      const data=await getJson(`/api/file?workspace=${escapePath(workspaceEl.value)}&path=${escapePath(node.path)}`);
      if(request===state.fileRequest)renderFile(data);
    }catch(error){
      if(request!==state.fileRequest)return;
      fileViewer.setAttribute('aria-busy','false');showCodeState('Preview unavailable',error.message);
    }
  }

  function closeFileViewer(immediate=false){
    if(fileViewer.hidden)return;
    state.fileRequest++;state.codeContent='';fileViewer.classList.remove('open');document.querySelector('.shell').inert=false;
    const finish=()=>{if(!fileViewer.classList.contains('open'))fileViewer.hidden=true};
    if(immediate)finish();else setTimeout(finish,180);
    if(state.lastFocus?.isConnected)state.lastFocus.focus({preventScroll:true});
  }

  async function loadWorkspaces(){
    state.workspaces=await getJson('/api/workspaces');
    workspaceEl.replaceChildren();
    for(const workspace of state.workspaces){
      const option=document.createElement('option'); option.value=workspace.id; option.textContent=workspace.name; workspaceEl.append(option);
    }
    const requested=new URLSearchParams(location.search).get('workspace');
    if(requested && state.workspaces.some(workspace=>workspace.id===requested)) workspaceEl.value=requested;
    if(!state.workspaces.length){ showEmpty('No workspaces yet','Open a repository in Workbench, then launch the map again.','W/'); return; }
    await loadTree(true);
  }

  async function loadTree(fit=false){
    if(!workspaceEl.value || state.refreshing) return;
    state.refreshing=true; $('refresh').disabled=true;
    if(!state.loadedOnce) showLoading();
    try{
      const data=await getJson(`/api/tree?workspace=${escapePath(workspaceEl.value)}`);
      const sameWorkspace=state.data&&state.data.workspace===data.workspace;
      const previousCollapsed=state.collapsed,previousSelection=state.selection;
      state.data=data; state.root=buildTree(data); state.selected=null; closeInspector();
      if(sameWorkspace){
        const valid=new Set(); walk(state.root,node=>{if(node.kind==='directory'&&previousCollapsed.has(node.path))valid.add(node.path)}); state.collapsed=valid;
        const selectable=new Set();walk(state.root,node=>{if(node.path)selectable.add(node.path)});state.selection=new Set([...previousSelection].filter(path=>selectable.has(path)));
      }else{
        state.collapsed=new Set();state.selection.clear();state.layers=[];state.viewMode='tree';state.architectureIndex=-1;state.architectureSelection=null;world.classList.remove('architecture-mode');$('architectureNav').hidden=true;
      }
      state.loadedOnce=true; emptyEl.hidden=true;
      render();
      if(fit) requestAnimationFrame(fitView);
      const when=new Date(); $('updated').textContent=`Updated ${when.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'})}`;
      const changed=data.entries.filter(entry=>entry.status).length;
      $('summary').textContent=`${data.entries.length.toLocaleString()} files · ${countDirectories(state.root).toLocaleString()} folders${changed?` · ${changed} changed`:''}${data.truncated?' · showing first 6,000':''}`;
      history.replaceState(null,'',`/?workspace=${escapePath(workspaceEl.value)}`);
      renderAgentNotes();
    }catch(error){ showEmpty('Could not map this repository',error.message,'!'); }
    finally{ state.refreshing=false; $('refresh').disabled=false; }
  }

  function buildTree(data){
    const root={name:data.name,path:'',kind:'root',children:[],depth:0,root:data.root};
    const directories=new Map([['',root]]);
    for(const entry of data.entries){
      const parts=entry.path.split('/'); let parent=root, current='';
      for(let i=0;i<parts.length-1;i++){
        current=current?`${current}/${parts[i]}`:parts[i];
        if(!directories.has(current)){
          const directory={name:parts[i],path:current,kind:'directory',children:[],depth:i+1};
          directories.set(current,directory); parent.children.push(directory);
        }
        parent=directories.get(current);
      }
      parent.children.push({...entry,children:[],depth:parts.length});
    }
    const sort=node=>{node.children.sort((a,b)=>(a.kind==='directory'?0:1)-(b.kind==='directory'?0:1)||a.name.localeCompare(b.name,undefined,{numeric:true,sensitivity:'base'}));node.children.forEach(sort)};
    const colorBranch=(node,branch)=>{node.branch=branch;node.children.forEach(child=>colorBranch(child,branch))};
    sort(root);root.children.forEach((node,index)=>colorBranch(node,index%branchPalette.length)); return root;
  }

  function walk(node,visit){ visit(node); node.children.forEach(child=>walk(child,visit)); }
  function countDirectories(node){ return node.children.reduce((sum,child)=>sum+(child.kind==='directory'?1:0)+countDirectories(child),0); }
  function searchable(node){ return `${node.name} ${node.path} ${node.extension||''}`.toLowerCase(); }
  function markMatches(node,query){
    const self=query && searchable(node).includes(query); let below=false;
    for(const child of node.children) below=markMatches(child,query)||below;
    node.matches=self; node.hasMatch=self||below; return node.hasMatch;
  }
  function visibleChildren(node){
    if(state.query) return node.children.filter(child=>child.hasMatch);
    return state.collapsed.has(node.path)?[]:node.children;
  }

  function render(){
    if(!state.root) return;
    const query=state.query.trim().toLowerCase(); markMatches(state.root,query);
    state.layout.clear();
    const top=visibleChildren(state.root), clusters=[], sectionPlans=[];
    const hubStart=PAD, hubX=PAD+ROOT_GAP;
    top.forEach((node,index)=>state.layout.set(node.path,{x:hubX,y:hubStart+index*HUB_STEP,node}));
    const rootY=top.length?hubStart+(top.length-1)*HUB_STEP/2:PAD;
    state.layout.set('__root__',{x:PAD,y:rootY,node:state.root});

    for(const hub of top){
      const levels=[];
      const collect=(node,depth)=>{
        for(const child of visibleChildren(node)){
          (levels[depth]||(levels[depth]=[])).push(child); collect(child,depth+1);
        }
      };
      collect(hub,0);
      const total=levels.reduce((sum,level)=>sum+(level?.length||0),0);
      if(!total) continue;

      const rowCap=Math.min(18,Math.max(5,Math.ceil(Math.sqrt(total*2.2))));
      let bandX=14, clusterWidth=0, clusterRows=1;
      const nodes=[];
      for(const level of levels){
        if(!level?.length) continue;
        const rows=Math.min(rowCap,level.length), columns=Math.ceil(level.length/rows);
        level.forEach((node,index)=>{
          const column=Math.floor(index/rows), row=index%rows;
          nodes.push({node,x:bandX+column*(NODE_W+GRID_X),y:14+row*(NODE_H+GRID_Y)});
        });
        const bandWidth=columns*NODE_W+(columns-1)*GRID_X;
        clusterWidth=Math.max(clusterWidth,bandX+bandWidth+14);
        clusterRows=Math.max(clusterRows,rows);
        bandX+=bandWidth+LEVEL_GAP;
      }
      const clusterHeight=clusterRows*NODE_H+(clusterRows-1)*GRID_Y;
      sectionPlans.push({hub,nodes,w:clusterWidth,h:clusterHeight+28,label:hub.name,branch:hub.branch});
    }

    const columnCount=Math.min(3,Math.max(1,Math.ceil(Math.sqrt(sectionPlans.length))));
    const clusterBaseX=hubX+NODE_W+LEVEL_GAP;
    let sectionY=PAD;
    for(let start=0;start<sectionPlans.length;start+=columnCount){
      const row=sectionPlans.slice(start,start+columnCount);
      const rowHeight=Math.max(...row.map(section=>section.h));
      let sectionX=clusterBaseX;
      for(const section of row){
        clusters.push({x:sectionX,y:sectionY,w:section.w,h:section.h,label:section.label,branch:section.branch});
        for(const item of section.nodes){
          state.layout.set(item.node.path,{x:sectionX+item.x,y:sectionY+item.y,node:item.node});
        }
        sectionX+=section.w+CLUSTER_COL_GAP;
      }
      sectionY+=rowHeight+CLUSTER_GAP;
    }

    const edges=[];
    for(const position of state.layout.values()){
      for(const child of visibleChildren(position.node)){
        const target=state.layout.get(child.path);
        if(target) edges.push([position,target]);
      }
    }
    const positions=[...state.layout.values()];
    const minX=Math.min(...positions.map(p=>p.x)), minY=Math.min(...positions.map(p=>p.y));
    const maxX=Math.max(...positions.map(p=>p.x+NODE_W)), maxY=Math.max(...positions.map(p=>p.y+NODE_H));
    state.bounds={x:minX-PAD,y:minY-PAD,w:maxX-minX+PAD*2,h:maxY-minY+PAD*2};
    renderClusters(clusters); renderEdges(edges); renderNodes(positions); renderOverlays(); renderAgentNotes(); updateToolbarUI(); updateSearchCount(positions);
    if(state.viewMode==='architecture'&&currentArchitecture())renderArchitecture();else{viewport.setAttribute('aria-label','Interactive repository file tree');world.classList.remove('architecture-mode');$('architectureNav').hidden=true;$('categorizeJob').setAttribute('aria-pressed','false');drawMinimap()}
  }

  function renderClusters(clusters){
    clustersEl.replaceChildren(); const fragment=document.createDocumentFragment();
    for(const cluster of clusters){
      const section=document.createElement('div'); section.className='cluster'; section.dataset.label=cluster.label;setBranchStyle(section,cluster.branch);
      section.style.transform=`translate(${cluster.x}px,${cluster.y}px)`; section.style.width=`${cluster.w}px`; section.style.height=`${cluster.h}px`; fragment.append(section);
    }
    clustersEl.append(fragment);
  }

  function renderEdges(edges){
    edgesEl.replaceChildren(); edgesEl.setAttribute('width',state.bounds.x+state.bounds.w); edgesEl.setAttribute('height',state.bounds.y+state.bounds.h);
    const fragment=document.createDocumentFragment();
    for(const [from,to] of edges){
      if(!to) continue; const x1=from.x+NODE_W,y1=from.y+NODE_H/2,x2=to.x,y2=to.y+NODE_H/2,curve=Math.max(34,(x2-x1)*.52);
      const path=document.createElementNS('http://www.w3.org/2000/svg','path');
      path.setAttribute('d',`M ${x1} ${y1} C ${x1+curve} ${y1}, ${x2-curve} ${y2}, ${x2} ${y2}`);
      path.setAttribute('class',`edge${state.query&&to.node.hasMatch?' active':''}`);setBranchStyle(path,to.node.branch);fragment.append(path);
    }
    edgesEl.append(fragment);
  }

  function renderNodes(positions){
    nodesEl.replaceChildren(); const fragment=document.createDocumentFragment();
    for(const {x,y,node} of positions){
      const button=document.createElement('button'); button.type='button';
      const key=node.path||'__root__'; const open=node.children.length&&!state.collapsed.has(node.path);
      button.className=`node ${node.kind}${open?' open':''}${node.matches?' match':''}${state.selected===key?' selected':''}${state.selection.has(key)?' multi-selected':''}`;
      button.style.transform=`translate(${x}px,${y}px)`; button.dataset.path=key; button.dataset.baseTitle=node.path||node.root;button.title=button.dataset.baseTitle;
      if(node.kind!=='root'){button.dataset.branch=String(node.branch||0);setBranchStyle(button,node.branch)}
      if(node.path)button.setAttribute('aria-pressed',String(state.selection.has(key)));
      const icon=document.createElement('span'); icon.className='node-icon'; icon.textContent=iconFor(node);
      const copy=document.createElement('span'); copy.className='node-copy';
      const name=document.createElement('span'); name.className='node-name'; name.textContent=node.name;
      const meta=document.createElement('span'); meta.className='node-meta'; meta.textContent=metaFor(node); copy.append(name,meta);
      const twist=document.createElement('span'); twist.className='twisty'; twist.textContent=node.children.length?'›':'';
      button.append(icon,copy,twist);
      if(node.status){const dot=document.createElement('span');dot.className=`status-dot ${node.status}`;dot.title=node.status;button.append(dot)}
      button.addEventListener('click',event=>{
        event.stopPropagation();
        if(node.path&&(state.selectMode||event.metaKey||event.ctrlKey||event.shiftKey)){
          toggleSelection(node.path);return;
        }
        if(node.kind==='file'||node.kind==='symlink'){openFile(node);return}
        selectNode(node);
        if(node.children.length){if(state.collapsed.has(node.path))state.collapsed.delete(node.path);else state.collapsed.add(node.path);render()}
      });
      fragment.append(button);
    }
    nodesEl.append(fragment);
  }

  function selectNode(node){
    state.selected=node.path||'__root__';
    $('inspectIcon').textContent=iconFor(node); $('inspectName').textContent=node.name; $('inspectKind').textContent=node.kind==='root'?'workspace root':node.kind;
    const details=[['Path',node.path||node.root],['Type',node.kind==='file'?(node.extension||'file'):node.kind],['Size',node.kind==='file'?formatBytes(node.size):'—'],['Modified',node.modified?new Date(node.modified*1000).toLocaleString():'—'],['Git status',node.status||'clean']];
    $('details').replaceChildren(...details.map(([term,value])=>{const wrap=document.createDocumentFragment(),dt=document.createElement('dt'),dd=document.createElement('dd');dt.textContent=term;dd.textContent=value;wrap.append(dt,dd);return wrap}));
    $('copyPath').hidden=node.kind==='root'; inspector.classList.add('open');
    for(const el of nodesEl.children) el.classList.toggle('selected',el.dataset.path===state.selected);
  }
  function closeInspector(){inspector.classList.remove('open');state.selected=null;for(const el of nodesEl.children)el.classList.remove('selected')}

  function toggleSelection(path){
    if(state.selection.has(path))state.selection.delete(path);else state.selection.add(path);
    const node=nodesEl.querySelector(`[data-path="${CSS.escape(path)}"]`);
    if(node){node.classList.toggle('multi-selected',state.selection.has(path));node.setAttribute('aria-pressed',String(state.selection.has(path)))}
    updateToolbarUI();
  }

  function clearSelection(){
    state.selection.clear();
    for(const node of nodesEl.children){node.classList.remove('multi-selected');if(node.dataset.path!=='__root__')node.setAttribute('aria-pressed','false')}
    updateToolbarUI();
  }

  function setSelectMode(enabled){
    state.selectMode=enabled;$('selectTool').setAttribute('aria-pressed',String(enabled));viewport.classList.toggle('selecting',enabled);
  }

  function updateToolbarUI(){
    const count=state.selection.size,suffix=count?` (${count} selected)`:'';
    $('analyzeJob').title=`Run the Analyze repository job${suffix}`;
    $('categorizeJob').title=`Run the Categorize repository job${suffix}`;
    $('agentTool').title=`Create an independent Claude agent note${suffix}`;
    $('categorizeJob').setAttribute('aria-pressed',String(state.viewMode==='architecture'));
  }

  function newNoteId(){
    if(crypto.randomUUID)return crypto.randomUUID();
    const bytes=crypto.getRandomValues(new Uint8Array(16));bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;
    const hex=[...bytes].map(byte=>byte.toString(16).padStart(2,'0')).join('');return `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`;
  }

  function currentCanvasCenter(){return{x:(viewport.clientWidth/2-state.x)/state.scale,y:(viewport.clientHeight/2-state.y)/state.scale}}
  function noteContextLabel(note){return note.paths.length?`${note.paths.length} selected · ${note.paths.slice(0,2).join(' · ')}${note.paths.length>2?' · …':''}`:'Whole repository'}
  function focusAgentNote(note){
    const next=Math.max(1,state.scale);state.scale=next;state.x=viewport.clientWidth/2-(note.x+180)*next;state.y=viewport.clientHeight/2-(note.y+180)*next;setTransform();
  }

  function createAgentNote({title='Agent note',paths=[...state.selection],prompt='',intent='analysis',run=false}={}){
    const center=currentCanvasCenter(),offset=(state.noteSerial++%5)*24;
    const note={id:newNoteId(),workspace:workspaceEl.value,title,x:center.x-180+offset,y:center.y-180+offset,paths:[...paths],turns:[],draft:run?'':prompt,pending:null,requestId:null,status:'ready',statusCopy:'Ready for a question',intent};
    state.notes.push(note);renderAgentNotes();focusAgentNote(note);
    if(run)askNote(note.id,prompt,intent);else requestAnimationFrame(()=>conversationNotes.querySelector(`[data-note-id="${CSS.escape(note.id)}"] .agent-note-prompt`)?.focus({preventScroll:true}));
    return note;
  }

  function removeAgentNote(id){state.notes=state.notes.filter(note=>note.id!==id);renderAgentNotes()}

  function beginNoteDrag(event,note,card,handle){
    if(event.button!==0||event.target.closest('button,input,textarea'))return;event.preventDefault();event.stopPropagation();
    const startX=event.clientX,startY=event.clientY,originX=note.x,originY=note.y;handle.setPointerCapture(event.pointerId);
    const move=moveEvent=>{if(moveEvent.pointerId!==event.pointerId)return;note.x=originX+(moveEvent.clientX-startX)/state.scale;note.y=originY+(moveEvent.clientY-startY)/state.scale;card.style.transform=`translate(${note.x}px,${note.y}px)`};
    const stop=stopEvent=>{if(stopEvent.pointerId!==event.pointerId)return;handle.removeEventListener('pointermove',move);handle.removeEventListener('pointerup',stop);handle.removeEventListener('pointercancel',stop)};
    handle.addEventListener('pointermove',move);handle.addEventListener('pointerup',stop);handle.addEventListener('pointercancel',stop);
  }

  function renderAgentNotes(){
    conversationNotes.replaceChildren();const fragment=document.createDocumentFragment();
    for(const note of state.notes.filter(note=>note.workspace===workspaceEl.value)){
      const card=document.createElement('article');card.className='agent-note';card.dataset.noteId=note.id;card.style.transform=`translate(${note.x}px,${note.y}px)`;
      const head=document.createElement('div');head.className='agent-note-head';
      const mark=document.createElement('div');mark.className='agent-note-mark';mark.textContent='✦';
      const title=document.createElement('div');title.className='agent-note-title';const strong=document.createElement('strong'),model=document.createElement('span');strong.textContent=note.title;model.textContent=CANVAS_AGENT_LABEL;title.append(strong,model);
      const close=document.createElement('button');close.className='tool agent-note-close';close.type='button';close.title='Remove note';close.setAttribute('aria-label',`Remove ${note.title}`);close.textContent='×';close.addEventListener('click',event=>{event.stopPropagation();removeAgentNote(note.id)});
      head.append(mark,title,close);head.addEventListener('pointerdown',event=>beginNoteDrag(event,note,card,head));
      const context=document.createElement('div');context.className='agent-note-context';context.textContent=noteContextLabel(note);context.title=note.paths.join('\n');
      const thread=document.createElement('div');thread.className='agent-thread';
      if(!note.turns.length&&!note.pending){const empty=document.createElement('div');empty.className='agent-note-empty';empty.textContent='This note is an independent agent. Ask a question here; its conversation stays only on this board while it is open.';thread.append(empty)}
      for(const turn of note.turns){const wrap=document.createElement('section'),question=document.createElement('div'),answer=document.createElement('pre');wrap.className='agent-turn';question.className='agent-question';question.textContent=turn.question;answer.className=`agent-answer${turn.error?' error':''}`;answer.textContent=turn.answer;wrap.append(question,answer);thread.append(wrap)}
      if(note.pending){const wrap=document.createElement('section'),question=document.createElement('div'),answer=document.createElement('pre');wrap.className='agent-turn';question.className='agent-question';question.textContent=note.pending;answer.className='agent-answer';answer.textContent=note.intent==='architecture'?'Categorizing the codebase into grounded areas…':'Reading the repository and preparing an answer…';wrap.append(question,answer);thread.append(wrap)}
      const status=document.createElement('div');status.className=`agent-note-status${note.status==='working'?' working':note.status==='error'?' error':''}`;status.textContent=note.statusCopy;
      const compose=document.createElement('div');compose.className='agent-note-compose';
      const prompt=document.createElement('textarea');prompt.className='agent-note-prompt';prompt.maxLength=4000;prompt.placeholder='Ask a follow-up…';prompt.value=note.draft;prompt.disabled=Boolean(note.requestId);prompt.setAttribute('aria-label',`Message ${note.title}`);prompt.addEventListener('input',()=>{note.draft=prompt.value;send.disabled=!prompt.value.trim()||Boolean(note.requestId)});prompt.addEventListener('keydown',event=>{if(event.key==='Enter'&&(event.metaKey||event.ctrlKey)){event.preventDefault();askNote(note.id)}});
      const send=document.createElement('button');send.className='tool agent-note-send';send.type='button';send.title='Send to a fresh Claude agent';send.setAttribute('aria-label','Send question');send.textContent='↗';send.disabled=!note.draft.trim()||Boolean(note.requestId);send.addEventListener('click',()=>askNote(note.id));compose.append(prompt,send);
      card.append(head,context,thread,status,compose);fragment.append(card);
    }
    conversationNotes.append(fragment);
    for(const thread of conversationNotes.querySelectorAll('.agent-thread'))thread.scrollTop=thread.scrollHeight;
  }

  async function askNote(noteId,explicitPrompt=null,intent=null){
    const note=state.notes.find(item=>item.id===noteId);if(!note||note.requestId)return;
    const prompt=(explicitPrompt??note.draft).trim();if(!prompt)return;note.intent=intent||'analysis';note.pending=prompt;note.draft='';note.status='working';note.statusCopy=note.intent==='architecture'?'Starting fresh categorization agent':'Starting fresh analysis agent';renderAgentNotes();
    const history=note.turns.filter(turn=>!turn.error).map(turn=>({question:turn.question,answer:turn.answer}));
    try{
      const exchange=await postJson('/api/ask',{workspace:note.workspace,note_id:note.id,scope:note.paths.length?'selection':'repository',intent:note.intent,prompt,paths:note.paths,history});
      note.requestId=exchange.id;note.statusCopy='Claude Code is working';renderAgentNotes();pollNote(note.id,exchange.id);
    }catch(error){note.turns.push({question:prompt,answer:error.message,error:true});note.pending=null;note.status='error';note.statusCopy='Could not start agent';renderAgentNotes()}
  }

  async function pollNote(noteId,requestId){
    const note=state.notes.find(item=>item.id===noteId);if(!note||note.requestId!==requestId)return;
    try{
      const exchange=await getJson(`/api/ask?id=${escapePath(requestId)}`);
      if(exchange.status==='complete'){
        note.turns.push({question:note.pending||exchange.prompt,answer:exchange.answer||'The agent added an explanation to the canvas.',error:false});note.pending=null;note.requestId=null;note.status='ready';note.statusCopy='Answered · ask a follow-up';
        const center={x:note.x+360,y:note.y+36};state.layers.push({id:`layer-${++state.layerSerial}`,origin:center,operations:exchange.operations||[]});renderOverlays();renderAgentNotes();
        if((exchange.operations||[]).some(operation=>operation.kind==='architecture')){state.viewMode='architecture';state.architectureIndex=architectureEntries().length-1;state.architectureSelection=null;renderArchitecture();requestAnimationFrame(fitView);updateToolbarUI()}
        return;
      }
      if(exchange.status==='error'){note.turns.push({question:note.pending||exchange.prompt,answer:exchange.error||'The request failed.',error:true});note.pending=null;note.requestId=null;note.status='error';note.statusCopy='Agent could not answer';renderAgentNotes();return}
      note.status='working';note.statusCopy=exchange.status==='queued'?'Waiting to start Claude Code':exchange.intent==='architecture'?'Categorizing repository':'Analyzing repository';renderAgentNotes();
    }catch(error){note.turns.push({question:note.pending||'Question',answer:error.message,error:true});note.pending=null;note.requestId=null;note.status='error';note.statusCopy='Connection lost';renderAgentNotes();return}
    setTimeout(()=>pollNote(noteId,requestId),850);
  }

  function runAnalyzeJob(){
    const selected=state.selection.size>0,prompt=selected?'Analyze these selected repository items. Explain their responsibilities, important execution paths, dependencies, and risks. Ground every claim in repository files.':'Analyze this repository. Explain its purpose, architecture, primary execution paths, dependencies, and the highest-risk or most important areas. Ground every claim in repository files.';
    createAgentNote({title:selected?'Selection analysis':'Repository analysis',prompt,intent:'analysis',run:true});
  }

  function openArchitectureAsk(paths=[...state.selection]){
    const selected=paths.length>0,prompt=selected?'Categorize this subsystem into a few meaningful components and show their important relationships.':'Categorize this repository into a few meaningful architectural areas and show their important relationships.';
    createAgentNote({title:selected?'Subsystem categories':'Repository categories',paths,prompt,intent:'architecture',run:true});
  }

  function overlayColor(color){return overlayColors[color]||overlayColors.green}
  function setOverlayColor(element,color){element.style.setProperty('--overlay-color',overlayColor(color))}
  function setMapColor(element,color){element.style.setProperty('--map-color',overlayColor(color))}
  function architectureEntries(){
    const entries=[];for(const layer of state.layers){for(const operation of layer.operations||[]){if(operation.kind==='architecture')entries.push(operation)}}return entries;
  }
  function currentArchitecture(){const entries=architectureEntries();return entries[state.architectureIndex]||null}

  function layoutArchitecture(map){
    state.architectureLayout.clear();const nodes=map.nodes||[],byId=new Map(nodes.map(node=>[node.id,node]));
    const outgoing=new Map(nodes.map(node=>[node.id,[]])),indegree=new Map(nodes.map(node=>[node.id,0]));
    for(const edge of map.edges||[]){if(!byId.has(edge.from)||!byId.has(edge.to))continue;outgoing.get(edge.from).push(edge.to);indegree.set(edge.to,indegree.get(edge.to)+1)}
    const ranks=new Map();
    if(!(map.edges||[]).length){const columns=Math.min(3,Math.max(1,Math.ceil(Math.sqrt(nodes.length))));nodes.forEach((node,index)=>ranks.set(node.id,index%columns))}
    else{
      let roots=nodes.filter(node=>indegree.get(node.id)===0);if(!roots.length&&nodes.length)roots=[nodes.reduce((best,node)=>outgoing.get(node.id).length>outgoing.get(best.id).length?node:best,nodes[0])];
      const queue=[];for(const root of roots){ranks.set(root.id,0);queue.push(root.id)}
      while(queue.length){const id=queue.shift(),rank=ranks.get(id);for(const next of outgoing.get(id)||[]){if(ranks.has(next))continue;ranks.set(next,rank+1);queue.push(next)}}
      let spare=0;for(const node of nodes){if(!ranks.has(node.id)){ranks.set(node.id,spare%Math.max(1,Math.min(3,nodes.length)));spare++}}
    }
    const columns=[];for(const node of nodes){const rank=ranks.get(node.id)||0;(columns[rank]||(columns[rank]=[])).push(node)}
    const maxRows=Math.max(1,...columns.map(column=>column?.length||0));
    for(let rank=0;rank<columns.length;rank++){
      const column=columns[rank]||[],offset=(maxRows-column.length)*(ARCH_H+52)/2;
      column.forEach((node,row)=>state.architectureLayout.set(node.id,{node,x:PAD+rank*(ARCH_W+170),y:PAD+offset+row*(ARCH_H+52)}));
    }
    const positions=[...state.architectureLayout.values()];
    if(!positions.length){state.bounds={x:0,y:0,w:1,h:1};return}
    const minX=Math.min(...positions.map(item=>item.x)),minY=Math.min(...positions.map(item=>item.y)),maxX=Math.max(...positions.map(item=>item.x+ARCH_W)),maxY=Math.max(...positions.map(item=>item.y+ARCH_H));
    state.bounds={x:minX-PAD,y:minY-PAD,w:maxX-minX+PAD*2,h:maxY-minY+PAD*2};
  }

  function renderArchitecture(){
    const map=currentArchitecture();if(!map){showTree();return}
    world.classList.add('architecture-mode');viewport.setAttribute('aria-label','Interactive repository architecture map');$('architectureNav').hidden=false;$('categorizeJob').setAttribute('aria-pressed','true');closeInspector();layoutArchitecture(map);
    const level=(map.level||'overview').replace('_',' ');$('architectureTitle').textContent=`${level} / ${map.title}`;$('architectureSummary').textContent=`AI-generated · ${(map.nodes||[]).length} concepts${map.summary?` · ${map.summary}`:''}`;
    const selected=(map.nodes||[]).find(node=>node.id===state.architectureSelection);if(!selected)state.architectureSelection=null;
    const backLabel=state.architectureIndex>0?'Previous architecture level':'Return to file tree';$('architectureBack').title=backLabel;$('architectureBack').setAttribute('aria-label',backLabel);$('architectureFiles').disabled=!selected;$('architectureDrill').disabled=!selected;
    architectureNodes.replaceChildren();architectureEdges.replaceChildren();
    const defs=document.createElementNS('http://www.w3.org/2000/svg','defs'),marker=document.createElementNS('http://www.w3.org/2000/svg','marker'),arrow=document.createElementNS('http://www.w3.org/2000/svg','path');marker.id='architectureArrow';marker.setAttribute('viewBox','0 0 10 10');marker.setAttribute('refX','9');marker.setAttribute('refY','5');marker.setAttribute('markerWidth','7');marker.setAttribute('markerHeight','7');marker.setAttribute('orient','auto-start-reverse');arrow.setAttribute('d','M 0 0 L 10 5 L 0 10 z');arrow.setAttribute('fill','context-stroke');marker.append(arrow);defs.append(marker);architectureEdges.append(defs);
    architectureEdges.setAttribute('width',Math.max(1,state.bounds.x+state.bounds.w+PAD));architectureEdges.setAttribute('height',Math.max(1,state.bounds.y+state.bounds.h+PAD));
    for(const edge of map.edges||[]){
      const from=state.architectureLayout.get(edge.from),to=state.architectureLayout.get(edge.to);if(!from||!to)continue;
      const forward=to.x>=from.x,x1=forward?from.x+ARCH_W:from.x,x2=forward?to.x:to.x+ARCH_W,y1=from.y+ARCH_H/2,y2=to.y+ARCH_H/2,direction=forward?1:-1,curve=Math.max(64,Math.abs(x2-x1)*.46);
      const path=document.createElementNS('http://www.w3.org/2000/svg','path');path.setAttribute('d',`M ${x1} ${y1} C ${x1+curve*direction} ${y1}, ${x2-curve*direction} ${y2}, ${x2} ${y2}`);path.setAttribute('class','architecture-edge');path.setAttribute('marker-end','url(#architectureArrow)');setMapColor(path,from.node.color);architectureEdges.append(path);
      if(edge.label){const label=document.createElementNS('http://www.w3.org/2000/svg','text');label.setAttribute('x',String((x1+x2)/2));label.setAttribute('y',String((y1+y2)/2-9));label.setAttribute('text-anchor','middle');label.setAttribute('class','architecture-edge-label');label.textContent=edge.label;architectureEdges.append(label)}
    }
    const fragment=document.createDocumentFragment();
    for(const {node,x,y} of state.architectureLayout.values()){
      const card=document.createElement('button'),eyebrow=document.createElement('span'),kind=document.createElement('span'),count=document.createElement('span'),title=document.createElement('span'),summary=document.createElement('span'),paths=document.createElement('span');card.type='button';card.className=`architecture-card${state.architectureSelection===node.id?' selected':''}`;card.style.transform=`translate(${x}px,${y}px)`;card.setAttribute('aria-pressed',String(state.architectureSelection===node.id));setMapColor(card,node.color);
      eyebrow.className='architecture-eyebrow';kind.textContent=node.kind||'component';count.textContent=`${(node.paths||[]).length} anchor${(node.paths||[]).length===1?'':'s'}`;eyebrow.append(kind,count);title.className='architecture-card-title';title.textContent=node.label;summary.className='architecture-card-copy';summary.textContent=node.summary||'Grounded repository concept';paths.className='architecture-paths';paths.textContent=(node.paths||[]).slice(0,3).join(' · ');card.append(eyebrow,title,summary,paths);
      card.addEventListener('click',event=>{event.stopPropagation();state.architectureSelection=state.architectureSelection===node.id?null:node.id;renderArchitecture()});fragment.append(card);
    }
    architectureNodes.append(fragment);drawMinimap();
  }

  function showArchitecture(index=architectureEntries().length-1){
    const entries=architectureEntries();if(!entries.length){openArchitectureAsk();return}state.viewMode='architecture';state.architectureIndex=Math.max(0,Math.min(index,entries.length-1));state.architectureSelection=null;setSelectMode(false);renderArchitecture();requestAnimationFrame(fitView);updateToolbarUI();
  }

  function fitPaths(paths){
    const positions=paths.map(path=>state.layout.get(path)).filter(Boolean);if(!positions.length){fitView();return}const pad=120,w=viewport.clientWidth,h=viewport.clientHeight,minX=Math.min(...positions.map(item=>item.x)),minY=Math.min(...positions.map(item=>item.y)),maxX=Math.max(...positions.map(item=>item.x+NODE_W)),maxY=Math.max(...positions.map(item=>item.y+NODE_H)),width=Math.max(NODE_W,maxX-minX),height=Math.max(NODE_H,maxY-minY);state.scale=Math.min(1.18,Math.max(.24,Math.min((w-pad*2)/width,(h-pad*2)/height)));state.x=(w-width*state.scale)/2-minX*state.scale;state.y=(h-height*state.scale)/2-minY*state.scale;setTransform();
  }

  function showTree(paths=[]){
    state.viewMode='tree';state.architectureSelection=null;viewport.setAttribute('aria-label','Interactive repository file tree');world.classList.remove('architecture-mode');$('architectureNav').hidden=true;$('categorizeJob').setAttribute('aria-pressed','false');if(paths.length)state.selection=new Set(paths);render();requestAnimationFrame(()=>paths.length?fitPaths(paths):fitView());updateToolbarUI();
  }

  function selectedArchitectureNode(){const map=currentArchitecture();return(map?.nodes||[]).find(node=>node.id===state.architectureSelection)||null}
  function useCategorizeJob(){if(state.viewMode==='architecture'){const selected=selectedArchitectureNode();openArchitectureAsk(selected?.paths||[])}else if(architectureEntries().length)showArchitecture();else openArchitectureAsk()}
  function architectureBack(){if(state.architectureIndex>0)showArchitecture(state.architectureIndex-1);else showTree()}
  function architectureFiles(){const selected=selectedArchitectureNode();if(selected)showTree(selected.paths||[])}
  function architectureDrill(){const selected=selectedArchitectureNode();if(selected)openArchitectureAsk(selected.paths||[])}
  function architectureRemap(){const map=currentArchitecture(),selected=selectedArchitectureNode(),paths=selected?.paths?.length?selected.paths:(map?.focus_paths||[]);openArchitectureAsk(paths)}
  function boundsFor(paths){
    const positions=paths.map(path=>state.layout.get(path)).filter(Boolean);if(!positions.length)return null;
    const minX=Math.min(...positions.map(item=>item.x)),minY=Math.min(...positions.map(item=>item.y));
    const maxX=Math.max(...positions.map(item=>item.x+NODE_W)),maxY=Math.max(...positions.map(item=>item.y+NODE_H));
    return{x:minX,y:minY,w:maxX-minX,h:maxY-minY};
  }

  function overlayAnchor(operation,layer,offset=0){
    const paths=operation.paths||[];const bounds=boundsFor(paths);
    if(bounds)return{x:bounds.x+bounds.w+22,y:bounds.y+offset};
    const finiteX=Number.isFinite(operation.x),finiteY=Number.isFinite(operation.y);
    return{x:finiteX?operation.x:layer.origin.x+offset,y:finiteY?operation.y:layer.origin.y+offset};
  }

  function renderOverlays(){
    for(const node of nodesEl.children){node.classList.remove('agent-highlight');node.style.removeProperty('--overlay-color');node.title=node.dataset.baseTitle||''}
    agentGroups.replaceChildren();agentEdges.replaceChildren();agentNotes.replaceChildren();agentDiagrams.replaceChildren();
    const defs=document.createElementNS('http://www.w3.org/2000/svg','defs'),marker=document.createElementNS('http://www.w3.org/2000/svg','marker'),arrow=document.createElementNS('http://www.w3.org/2000/svg','path');
    marker.id='agentArrow';marker.setAttribute('viewBox','0 0 10 10');marker.setAttribute('refX','9');marker.setAttribute('refY','5');marker.setAttribute('markerWidth','7');marker.setAttribute('markerHeight','7');marker.setAttribute('orient','auto-start-reverse');arrow.setAttribute('d','M 0 0 L 10 5 L 0 10 z');arrow.setAttribute('fill','context-stroke');marker.append(arrow);defs.append(marker);agentEdges.append(defs);
    agentEdges.setAttribute('width',Math.max(1,state.bounds.x+state.bounds.w+1000));agentEdges.setAttribute('height',Math.max(1,state.bounds.y+state.bounds.h+1000));
    let noteOffset=0,diagramOffset=0;
    for(const layer of state.layers){
      for(const operation of layer.operations||[]){
        if(operation.kind==='highlight'){
          for(const path of operation.paths||[]){const node=nodesEl.querySelector(`[data-path="${CSS.escape(path)}"]`);if(node){node.classList.add('agent-highlight');setOverlayColor(node,operation.color);if(operation.label)node.title=`${node.title}\n${operation.label}`}}
        }else if(operation.kind==='group'){
          const bounds=boundsFor(operation.paths||[]);if(!bounds)continue;
          const group=document.createElement('div'),label=document.createElement('span');group.className='agent-group';label.textContent=operation.title;group.append(label);setOverlayColor(group,operation.color);
          group.style.transform=`translate(${bounds.x-10}px,${bounds.y-10}px)`;group.style.width=`${bounds.w+20}px`;group.style.height=`${bounds.h+20}px`;agentGroups.append(group);
        }else if(operation.kind==='connect'){
          const from=state.layout.get(operation.from),to=state.layout.get(operation.to);if(!from||!to)continue;
          const x1=from.x+NODE_W,y1=from.y+NODE_H/2,x2=to.x,y2=to.y+NODE_H/2,curve=Math.max(38,Math.abs(x2-x1)*.45);
          const path=document.createElementNS('http://www.w3.org/2000/svg','path');path.setAttribute('d',`M ${x1} ${y1} C ${x1+curve} ${y1}, ${x2-curve} ${y2}, ${x2} ${y2}`);path.setAttribute('class','agent-connector');path.setAttribute('marker-end','url(#agentArrow)');setOverlayColor(path,operation.color);agentEdges.append(path);
          if(operation.label){const label=document.createElementNS('http://www.w3.org/2000/svg','text');label.setAttribute('x',String((x1+x2)/2));label.setAttribute('y',String((y1+y2)/2-7));label.setAttribute('text-anchor','middle');label.setAttribute('class','agent-edge-label');label.textContent=operation.label;agentEdges.append(label)}
        }else if(operation.kind==='note'){
          const anchor=overlayAnchor(operation,layer,noteOffset);noteOffset+=18;
          const note=document.createElement('article'),title=document.createElement('strong'),body=document.createElement('p');note.className='canvas-callout';title.textContent=operation.title;body.textContent=operation.body;note.append(title,body);setOverlayColor(note,operation.color);
          note.style.transform=`translate(${anchor.x}px,${anchor.y}px)`;agentNotes.append(note);
        }else if(operation.kind==='diagram'){
          const anchor={x:layer.origin.x+diagramOffset,y:layer.origin.y+diagramOffset};diagramOffset+=28;
          const card=document.createElement('section'),title=document.createElement('strong'),nodes=document.createElement('div'),edges=document.createElement('div');card.className='agent-diagram';title.textContent=operation.title;nodes.className='diagram-nodes';edges.className='diagram-edges';
          for(const item of operation.nodes||[]){const node=document.createElement('div');node.className='diagram-node';node.textContent=item.label;if(item.path){node.dataset.path=item.path;node.title=item.path}nodes.append(node)}
          for(const item of operation.edges||[]){const edge=document.createElement('div');edge.textContent=`${item.from} → ${item.to}${item.label?` · ${item.label}`:''}`;edges.append(edge)}
          card.append(title,nodes,edges);card.style.transform=`translate(${anchor.x}px,${anchor.y}px)`;agentDiagrams.append(card);
        }
      }
    }
    $('layerTools').hidden=!state.layers.length;$('undoLayer').disabled=!state.layers.length;$('clearLayers').disabled=!state.layers.length;
  }

  function setTransform(){
    world.style.transform=`translate(${state.x}px,${state.y}px) scale(${state.scale})`;
    drawMinimap();
  }
  function zoomAt(next,cx=viewport.clientWidth/2,cy=viewport.clientHeight/2){
    next=Math.min(2.2,Math.max(.18,next)); const wx=(cx-state.x)/state.scale,wy=(cy-state.y)/state.scale;
    state.x=cx-wx*next; state.y=cy-wy*next; state.scale=next; setTransform();
  }
  function fitView(){
    if(!state.layout.size)return; const w=viewport.clientWidth,h=viewport.clientHeight,pad=70;
    state.scale=Math.min(1.08,Math.max(.18,Math.min((w-pad*2)/state.bounds.w,(h-pad*2)/state.bounds.h)));
    state.x=(w-state.bounds.w*state.scale)/2-state.bounds.x*state.scale; state.y=(h-state.bounds.h*state.scale)/2-state.bounds.y*state.scale; setTransform();
  }

  function minimapMetrics(){
    const canvas=$('minimap'),rect=canvas.getBoundingClientRect(),pad=12,scale=Math.min((rect.width-pad*2)/state.bounds.w,(rect.height-pad*2)/state.bounds.h);
    return{canvas,rect,pad,scale,ox:pad-state.bounds.x*scale,oy:pad-state.bounds.y*scale};
  }

  function minimapViewport(metrics){
    const x=metrics.ox+(-state.x/state.scale)*metrics.scale,y=metrics.oy+(-state.y/state.scale)*metrics.scale,w=viewport.clientWidth/state.scale*metrics.scale,h=viewport.clientHeight/state.scale*metrics.scale;
    return{x,y,w,h,cx:x+w/2,cy:y+h/2};
  }

  function clampWorldCenter(value,start,size,visible){
    if(visible>=size)return start+size/2;const half=visible/2;return Math.min(start+size-half,Math.max(start+half,value));
  }

  function centerCanvasAtWorld(wx,wy){
    const visibleW=viewport.clientWidth/state.scale,visibleH=viewport.clientHeight/state.scale;
    wx=clampWorldCenter(wx,state.bounds.x,state.bounds.w,visibleW);wy=clampWorldCenter(wy,state.bounds.y,state.bounds.h,visibleH);
    state.x=viewport.clientWidth/2-wx*state.scale;state.y=viewport.clientHeight/2-wy*state.scale;setTransform();
  }

  function navigateFromMinimap(event){
    const drag=state.minimapDrag;if(!drag||drag.pointer!==event.pointerId)return;const metrics=minimapMetrics(),localX=event.clientX-metrics.rect.left-drag.offsetX,localY=event.clientY-metrics.rect.top-drag.offsetY;
    centerCanvasAtWorld((localX-metrics.ox)/metrics.scale,(localY-metrics.oy)/metrics.scale);
  }

  function drawMinimap(){
    const canvas=$('minimap'),ctx=canvas.getContext('2d'); if(!ctx||!state.layout.size)return;
    const metrics=minimapMetrics(),dpr=Math.min(2,window.devicePixelRatio||1),w=Math.round(metrics.rect.width*dpr),h=Math.round(metrics.rect.height*dpr);
    if(canvas.width!==w||canvas.height!==h){canvas.width=w;canvas.height=h}ctx.setTransform(dpr,0,0,dpr,0,0);ctx.clearRect(0,0,metrics.rect.width,metrics.rect.height);
    if(state.viewMode==='architecture')for(const p of state.architectureLayout.values()){ctx.fillStyle=mapCanvasColors[p.node.color]||mapCanvasColors.green;ctx.fillRect(metrics.ox+p.x*metrics.scale,metrics.oy+p.y*metrics.scale,Math.max(2,ARCH_W*metrics.scale),Math.max(2,ARCH_H*metrics.scale))}
    else for(const p of state.layout.values()){ctx.fillStyle=p.node.kind==='root'?'#38a852':branchPalette[p.node.branch||0].canvas;ctx.fillRect(metrics.ox+p.x*metrics.scale,metrics.oy+p.y*metrics.scale,Math.max(1.5,NODE_W*metrics.scale),Math.max(1.5,NODE_H*metrics.scale))}
    const frame=minimapViewport(metrics);ctx.fillStyle='rgba(56,168,82,.12)';ctx.fillRect(frame.x,frame.y,frame.w,frame.h);ctx.strokeStyle='rgba(255,255,255,.92)';ctx.lineWidth=3;ctx.strokeRect(frame.x,frame.y,frame.w,frame.h);ctx.strokeStyle='#268d49';ctx.lineWidth=1.5;ctx.strokeRect(frame.x,frame.y,frame.w,frame.h);
  }

  function updateSearchCount(positions){
    if(!state.query){$('searchCount').textContent='';return}
    const count=positions.filter(position=>position.node.matches).length; $('searchCount').textContent=`${count} match${count===1?'':'es'}`;
  }
  function showLoading(){emptyEl.hidden=false;emptyEl.querySelector('.glyph').className='glyph spinner';emptyEl.querySelector('.glyph').textContent='◌';emptyEl.querySelector('h2').textContent='Mapping repository';emptyEl.querySelector('p').textContent='Reading the file tree and arranging the canvas.'}
  function showEmpty(title,message,glyph){emptyEl.hidden=false;emptyEl.querySelector('.glyph').className='glyph';emptyEl.querySelector('.glyph').textContent=glyph;emptyEl.querySelector('h2').textContent=title;emptyEl.querySelector('p').textContent=message}

  function beginLasso(event){
    const rect=viewport.getBoundingClientRect(),x=event.clientX-rect.left,y=event.clientY-rect.top;
    state.lasso={pointer:event.pointerId,x,y,currentX:x,currentY:y,additive:event.metaKey||event.ctrlKey};
    const box=$('lassoBox');box.hidden=false;box.style.left=`${x}px`;box.style.top=`${y}px`;box.style.width='0';box.style.height='0';viewport.setPointerCapture(event.pointerId);
  }
  function moveLasso(event){
    if(!state.lasso||state.lasso.pointer!==event.pointerId)return;const rect=viewport.getBoundingClientRect();state.lasso.currentX=event.clientX-rect.left;state.lasso.currentY=event.clientY-rect.top;
    const left=Math.min(state.lasso.x,state.lasso.currentX),top=Math.min(state.lasso.y,state.lasso.currentY),width=Math.abs(state.lasso.currentX-state.lasso.x),height=Math.abs(state.lasso.currentY-state.lasso.y),box=$('lassoBox');box.style.left=`${left}px`;box.style.top=`${top}px`;box.style.width=`${width}px`;box.style.height=`${height}px`;
  }
  function finishLasso(cancelled=false){
    const lasso=state.lasso;if(!lasso)return;state.lasso=null;$('lassoBox').hidden=true;if(cancelled)return;
    const viewportRect=viewport.getBoundingClientRect(),left=viewportRect.left+Math.min(lasso.x,lasso.currentX),right=viewportRect.left+Math.max(lasso.x,lasso.currentX),top=viewportRect.top+Math.min(lasso.y,lasso.currentY),bottom=viewportRect.top+Math.max(lasso.y,lasso.currentY);
    if(!lasso.additive)state.selection.clear();
    for(const node of nodesEl.children){if(node.dataset.path==='__root__')continue;const rect=node.getBoundingClientRect(),hit=rect.right>=left&&rect.left<=right&&rect.bottom>=top&&rect.top<=bottom;if(hit)state.selection.add(node.dataset.path);node.classList.toggle('multi-selected',state.selection.has(node.dataset.path));node.setAttribute('aria-pressed',String(state.selection.has(node.dataset.path)))}
    updateToolbarUI();
  }

  viewport.addEventListener('pointerdown',event=>{
    if(event.button!==0||event.target.closest('.node,#inspector,.agent-note,.canvas-callout,.agent-diagram,.architecture-card,#architectureNav,#minimap,#layerTools'))return;
    closeInspector();
    if(state.selectMode||event.shiftKey){beginLasso(event);return}
    state.drag={pointer:event.pointerId,x:event.clientX,y:event.clientY,ox:state.x,oy:state.y};viewport.setPointerCapture(event.pointerId);viewport.classList.add('panning')
  });
  viewport.addEventListener('pointermove',event=>{if(state.lasso){moveLasso(event);return}if(!state.drag||state.drag.pointer!==event.pointerId)return;state.x=state.drag.ox+event.clientX-state.drag.x;state.y=state.drag.oy+event.clientY-state.drag.y;setTransform()});
  const stopPointer=event=>{if(state.lasso&&(!event||state.lasso.pointer===event.pointerId)){finishLasso(event?.type==='pointercancel');return}state.drag=null;viewport.classList.remove('panning')};viewport.addEventListener('pointerup',stopPointer);viewport.addEventListener('pointercancel',stopPointer);
  viewport.addEventListener('wheel',event=>{event.preventDefault();if(event.ctrlKey||event.metaKey){zoomAt(state.scale*Math.exp(-event.deltaY*.002),event.clientX-viewport.getBoundingClientRect().left,event.clientY-viewport.getBoundingClientRect().top)}else{state.x-=event.deltaX;state.y-=event.deltaY;setTransform()}},{passive:false});
  viewport.addEventListener('keydown',event=>{if(event.target.closest('input,textarea,select,button,[contenteditable="true"]'))return;if(event.key==='f'||event.key==='F'){fitView()}else if(event.key==='+'||event.key==='='){zoomAt(state.scale*1.18)}else if(event.key==='-'){zoomAt(state.scale/1.18)}else if(event.key==='Escape'){closeInspector();if(state.selection.size)clearSelection();else setSelectMode(false)}else{return}event.preventDefault()});
  const minimap=$('minimap');
  minimap.addEventListener('pointerdown',event=>{
    if(event.button!==0)return;event.preventDefault();event.stopPropagation();minimap.focus({preventScroll:true});const metrics=minimapMetrics(),frame=minimapViewport(metrics),x=event.clientX-metrics.rect.left,y=event.clientY-metrics.rect.top,inside=x>=frame.x&&x<=frame.x+frame.w&&y>=frame.y&&y<=frame.y+frame.h;
    state.minimapDrag={pointer:event.pointerId,offsetX:inside?x-frame.cx:0,offsetY:inside?y-frame.cy:0};minimap.setPointerCapture(event.pointerId);minimap.classList.add('navigating');navigateFromMinimap(event);
  });
  minimap.addEventListener('pointermove',event=>{if(!state.minimapDrag)return;event.preventDefault();event.stopPropagation();navigateFromMinimap(event)});
  const stopMinimap=event=>{if(!state.minimapDrag||event&&state.minimapDrag.pointer!==event.pointerId)return;if(event){event.preventDefault();event.stopPropagation()}state.minimapDrag=null;minimap.classList.remove('navigating')};minimap.addEventListener('pointerup',stopMinimap);minimap.addEventListener('pointercancel',stopMinimap);
  minimap.addEventListener('wheel',event=>{event.preventDefault();event.stopPropagation()},{passive:false});
  minimap.addEventListener('keydown',event=>{
    const centerX=(viewport.clientWidth/2-state.x)/state.scale,centerY=(viewport.clientHeight/2-state.y)/state.scale,stepX=viewport.clientWidth/state.scale*.22,stepY=viewport.clientHeight/state.scale*.22;let x=centerX,y=centerY;
    if(event.key==='ArrowLeft')x-=stepX;else if(event.key==='ArrowRight')x+=stepX;else if(event.key==='ArrowUp')y-=stepY;else if(event.key==='ArrowDown')y+=stepY;else if(event.key==='Home'||event.key==='Enter'){event.preventDefault();fitView();return}else{return}event.preventDefault();centerCanvasAtWorld(x,y);
  });
  workspaceEl.addEventListener('change',()=>{closeFileViewer(true);searchEl.value='';state.query='';clearSelection();state.layers=[];state.viewMode='tree';state.architectureIndex=-1;renderOverlays();loadTree(true)});
  searchEl.addEventListener('input',()=>{state.query=searchEl.value;if(state.viewMode==='architecture'){state.viewMode='tree';state.architectureSelection=null}render();if(state.query)requestAnimationFrame(fitView)});
  $('refresh').addEventListener('click',()=>loadTree(false)); $('zoomIn').addEventListener('click',()=>zoomAt(state.scale*1.18)); $('zoomOut').addEventListener('click',()=>zoomAt(state.scale/1.18)); $('fit').addEventListener('click',fitView);
  $('selectTool').addEventListener('click',()=>setSelectMode(!state.selectMode));$('analyzeJob').addEventListener('click',runAnalyzeJob);$('categorizeJob').addEventListener('click',useCategorizeJob);$('agentTool').addEventListener('click',()=>createAgentNote());
  $('architectureBack').addEventListener('click',architectureBack);$('architectureFiles').addEventListener('click',architectureFiles);$('architectureDrill').addEventListener('click',architectureDrill);$('architectureRemap').addEventListener('click',architectureRemap);
  $('undoLayer').addEventListener('click',()=>{state.layers.pop();const maps=architectureEntries();if(state.viewMode==='architecture'){if(maps.length)showArchitecture(Math.min(state.architectureIndex,maps.length-1));else showTree()}renderOverlays();updateToolbarUI()});
  $('clearLayers').addEventListener('click',()=>{state.layers=[];if(state.viewMode==='architecture')showTree();renderOverlays();updateToolbarUI()});
  $('closeInspector').addEventListener('click',closeInspector);
  $('closeCode').addEventListener('click',()=>closeFileViewer());
  fileViewer.addEventListener('click',event=>{if(event.target===fileViewer)closeFileViewer()});
  document.addEventListener('keydown',event=>{
    if(fileViewer.hidden)return;
    if(event.key==='Escape'){event.preventDefault();closeFileViewer();return}
    if(event.key==='Tab'){
      const focusable=[$('copyCode'),$('closeCode'),$('codeSurface')],first=focusable[0],last=focusable[focusable.length-1];
      if(event.shiftKey&&document.activeElement===first){event.preventDefault();last.focus()}
      else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first.focus()}
    }
  });
  $('copyCode').addEventListener('click',async()=>{
    if(!state.codeContent)return;
    await navigator.clipboard.writeText(state.codeContent);const label=$('copyCode').querySelector('span');label.textContent='Copied';setTimeout(()=>label.textContent='Copy',1200);
  });
  $('copyPath').addEventListener('click',async()=>{const position=state.layout.get(state.selected);if(!position)return;await navigator.clipboard.writeText(position.node.path);const button=$('copyPath');const old=button.textContent;button.textContent='Copied';setTimeout(()=>button.textContent=old,1200)});
  window.addEventListener('resize',()=>{setTransform()});
  setInterval(()=>{if(!document.hidden&&state.loadedOnce)loadTree(false)},15000);
  loadWorkspaces().catch(error=>showEmpty('Could not start repository map',error.message,'!'));
})();
</script>
</body>
</html>"###;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_contains_the_core_canvas_controls() {
        for id in [
            "viewport",
            "workspace",
            "search",
            "refresh",
            "fit",
            "minimap",
            "fileViewer",
            "codeLines",
            "closeCode",
            "selectTool",
            "agentEdges",
            "conversationNotes",
            "agentTool",
            "analyzeJob",
            "categorizeJob",
            "layerTools",
            "architectureView",
            "architectureNav",
            "architectureFiles",
            "architectureDrill",
        ] {
            assert!(HTML.contains(&format!("id=\"{id}\"")), "missing {id}");
        }
        assert!(HTML.contains("pointerdown"));
        assert!(HTML.contains("wheel"));
        assert!(HTML.contains("id=\"clusters\""));
        assert!(HTML.contains("const top=visibleChildren(state.root)"));
        assert!(!HTML.contains("collapseInitial(state.root)"));
        assert!(HTML.contains("/api/file?workspace="));
        assert!(HTML.contains("/api/ask"));
        assert!(HTML.contains("beginLasso"));
        assert!(HTML.contains("renderOverlays"));
        assert!(HTML.contains("event.target.closest('input,textarea,select,button"));
        assert!(HTML.contains("scope:note.paths.length?'selection':'repository'"));
        assert!(HTML.contains("note_id:note.id"));
        assert!(HTML.contains("Claude Code · Sonnet 5"));
        assert!(HTML.contains("function renderAgentNotes()"));
        assert!(HTML.contains("function runAnalyzeJob()"));
        assert!(HTML.contains("function useCategorizeJob()"));
        assert!(!HTML.contains("localStorage"));
        assert!(HTML.contains("function renderArchitecture()"));
        assert!(HTML.contains("function navigateFromMinimap(event)"));
        assert!(HTML.contains("minimap.addEventListener('pointerdown'"));
        assert!(HTML.contains("minimap.addEventListener('keydown'"));
        assert!(HTML.contains("Click or drag to navigate"));
        assert!(HTML.contains("AI-generated"));
        assert!(HTML.contains("color-scheme:light"));
        assert!(!HTML.contains("radial-gradient"));
    }
}
