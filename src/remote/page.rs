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
  /* Served from this binary, not from Google — see `server::font_for`. */
  @font-face {
    font-family:"IBM Plex Mono"; font-style:normal; font-weight:400;
    font-display:swap; src:url("/font/ibm-plex-mono-400.woff2") format("woff2");
  }
  @font-face {
    font-family:"IBM Plex Mono"; font-style:normal; font-weight:500;
    font-display:swap; src:url("/font/ibm-plex-mono-500.woff2") format("woff2");
  }

  /* One palette per line, twice over: CSS cannot name a set of custom
     properties and apply it from two selectors, and the toggle needs to beat
     the system preference in both directions. */
  :root {
    color-scheme:dark;
    --bg:#0c0e13; --surface:#171a21; --raised:#1e222b; --line:#272c37;
    --fg:#e8eaf0; --dim:#8b93a4; --faint:#5d6575;
    --accent:#4361ee; --on-accent:#fff;
    --warn:hsl(20 88% 52%); --warn-bg:hsl(20 40% 13%); --ok:hsl(150 62% 40%);
    --shadow:0 1px 2px #0000004d;
    /* Fields and code wells recede from the surface they sit on. Which
       direction that is depends on the theme, so it cannot be derived. */
    --field:#0c0e13; --edge:#2b313d; --chrome:#12151b;
  }
  @media (prefers-color-scheme: light) {
    :root:not([data-theme="dark"]) {
      color-scheme:light;
      --bg:#f2f3f7; --surface:#fff; --raised:#fff; --line:#e3e6ec;
      --fg:#14161b; --dim:#5f6779; --faint:#98a0b0;
      --accent:#3355e8; --on-accent:#fff;
      --warn:hsl(20 88% 34%); --warn-bg:hsl(38 80% 94%); --ok:hsl(150 62% 26%);
      --shadow:0 1px 2px #10121a14, 0 1px 1px #10121a0f;
      --field:#f2f3f7; --edge:#e6e9ef; --chrome:#fbfbfd;
    }
  }
  :root[data-theme="light"], .t-light {
    color-scheme:light;
    --bg:#f2f3f7; --page:var(--bg); --stops:var(--no-stops); --surface:#fff; --raised:#fff; --line:#e3e6ec;
    --fg:#14161b; --dim:#5f6779; --faint:#98a0b0;
    --accent:#3355e8; --on-accent:#fff;
    --warn:#a96a00; --warn-bg:#fdf5e6; --ok:#1f8a4c;
    --shadow:0 1px 2px #10121a14, 0 1px 1px #10121a0f;
    --field:#f2f3f7;
  }

  /* One hue throughout, differentiated by lightness. Everything below is
     hsl() of the same angle as the colour it is named for, so a shade is a
     step rather than a guess. */

  /* 藤黄 — #FFBA00 at hsl(44 100% 50%), and black on it is 12.3:1, the most
     headroom any of these has. (White is 1.7:1, so black is not a preference
     here; it is the only pairing that works.)

     Differentiated by *saturation*, which the orange this replaces took three
     goes to arrive at. Lifting surfaces toward white goes pastel and the
     colour stops being the colour; darkening them holds the mood but eats the
     contrast black needs. Chroma is the axis with room in it: the page stays
     fully saturated, cards are the same hue muted to ochre — near enough in
     lightness to keep black at 8.6:1, far enough in colour to read as a
     different material. */
  :root[data-theme="gamboge"], .t-gamboge {
    color-scheme:light;
    --bg:#ffba00; --page:var(--bg); --stops:var(--no-stops);
    --surface:hsl(44 52% 52%); --raised:hsl(44 45% 46%); --line:hsl(44 45% 32%);
    --fg:hsl(44 70% 7%); --dim:hsl(44 60% 16%); --faint:hsl(44 45% 24%);
    --accent:hsl(44 95% 22%); --on-accent:#fff;
    --warn:hsl(350 88% 28%); --warn-bg:hsl(44 45% 62%); --ok:hsl(150 62% 18%);
    --shadow:0 1px 3px hsl(44 70% 12% / .3);
    --field:hsl(44 42% 48%); --edge:hsl(44 42% 38%); --chrome:hsl(44 45% 50%);
  }

  /* 曙色 into 象牙色 — dawn falling to ivory down the page.
     
     A gradient breaks the assumption every other theme rests on: that a
     surface can be a colour. Luminance runs 0.35 at the top to 0.83 at the
     bottom, a 2.3x span, so a card mixed to look right on the salmon is wrong
     by the time it reaches the ivory. The surfaces here are *washes* instead
     — black at a few percent — so each one tints whatever it happens to be
     sitting on and holds the same relationship all the way down.
     
     Black text spans 8.1:1 at the top to 17.6:1 at the bottom, so it is
     comfortable throughout. White would be 2.6:1 at its best, so it is not a
     choice. The drawer is opaque: it slides over the page, and a wash would
     show the conversation through it. */
  :root[data-theme="dawn"], .t-dawn {
    color-scheme:light;
    --bg:#fa7b62; --page:var(--bg);
    --stops:#fa7b62 0%,#f6a88f 42%,#f3ead7 100%;
    --surface:#0000001a; --raised:#00000026; --line:#0000003d;
    --fg:hsl(12 50% 9%); --dim:hsl(12 38% 18%); --faint:hsl(12 32% 19%);
    --accent:hsl(9 65% 32%); --on-accent:#fff;
    --warn:hsl(350 88% 16%); --warn-bg:#00000014; --ok:hsl(150 62% 10%);
    --shadow:0 1px 3px hsl(12 50% 20% / .22);
    --field:#0000001f; --edge:#0000002e; --chrome:#f7e7da;
  }

  /* 藤黄 → 象牙色 → 乳白色: light draining out of the page from top to bottom.
     
     Three stops rather than two, which mostly changes where the constraint
     sits. The gamboge end is the darkest thing here at 0.56 luminance and the
     milk white end is 0.90, so black is measured against the top and is
     comfortable everywhere below it by definition — 12.3:1 at the worst
     point, 18.9:1 at the best. Washes again for the surfaces, for the same
     reason as the dawn theme: no single card colour is right across a span
     that wide.
     
     This is the solid gamboge with the light let out of it. Both are kept
     because they are not the same room. */
  :root[data-theme="milk"], .t-milk {
    color-scheme:light;
    --bg:#ffba00; --page:var(--bg);
    --stops:#ffba00 0%,#f3ead7 52%,#f3f3f3 100%;
    --surface:#00000014; --raised:#00000021; --line:#00000038;
    --fg:hsl(44 70% 7%); --dim:hsl(44 55% 16%); --faint:hsl(44 45% 21%);
    --accent:hsl(40 95% 24%); --on-accent:#fff;
    --warn:hsl(350 88% 28%); --warn-bg:#0000000f; --ok:hsl(150 62% 18%);
    --shadow:0 1px 3px hsl(44 70% 12% / .2);
    --field:#0000001a; --edge:#00000029; --chrome:#f7f1e6;
  }

  /* 朱華絹 / 藍花鼠 / 冬暮鼠 — a conic gradient, blurred, which is the first
     theme here whose background is not a ramp between two known ends.
     
     All three colours are mid-tone: luminance 0.19 to 0.37. Black clears
     every one of them (4.8:1 at worst) but only just, and "only just" is the
     problem — at 4.8:1 there is no room left underneath for a muted tier, so
     `--dim` and `--faint` would have to be black too and stop being muted.
     White is worse: 2.5:1 on the silk.
     
     Hence the scrim. A 22% white veil over the whole thing lifts the darkest
     corner from 4.8:1 to 7.1:1, which buys back the room the text scale needs
     while leaving all three colours plainly themselves — silk stays silk. It
     is the same move as the washes elsewhere, pointed the other way. */
  :root[data-theme="dusk"], .t-dusk {
    color-scheme:light;
    --bg:#a9a3ab; --page:var(--bg);
    --bg:#a9a3ab;
    --stops:#c89888 0%,#c89888 26%,#4b80ea 52%,#6a7a88 80%,#c89888 100%;
    --veil:#ffffff38;
    --gradient-default:angular;
    --surface:#0000001a; --raised:#00000026; --line:#00000042;
    --fg:hsl(215 40% 8%); --dim:hsl(215 28% 16%); --faint:hsl(215 22% 21%);
    --accent:hsl(219 70% 30%); --on-accent:#fff;
    --warn:hsl(20 88% 14%); --warn-bg:#00000014; --ok:hsl(150 62% 12%);
    --shadow:0 1px 3px hsl(215 40% 15% / .25);
    --field:#0000001f; --edge:#00000030; --chrome:#c3c0c6;
  }

  /* 濡羽色 → 銀朱. The first dark gradient here, which inverts the surface
     model wholesale: every other gradient theme washes its cards with black
     over a light page, and this one washes with white over a dark one.
     
     Cinnabar is the constraint, not the black — white on it is 5.9:1, fine
     for the text but leaving almost nothing underneath, so the muted tier
     could barely dim before failing. A 20% black veil takes it to 8.2:1 and
     gives the tiers somewhere to go, the same move dusk makes in the other
     direction. */
  :root[data-theme="cinnabar"], .t-cinnabar {
    color-scheme:dark;
    --bg:#0c1021; --page:var(--bg);
    --stops:#0c1021 0%,#4a1220 55%,#bc2d29 100%;
    --veil:#00000033;
    --surface:#ffffff14; --raised:#ffffff21; --line:#ffffff3d;
    --fg:#fff; --dim:hsl(6 25% 90%); --faint:hsl(6 20% 84%);
    --accent:#bc2d29; --on-accent:#fff;
    --warn:hsl(40 95% 72%); --warn-bg:#ffffff14; --ok:hsl(150 60% 68%);
    --busy:hsl(200 95% 72%);
    --shadow:0 1px 3px #00000066;
    --field:#ffffff1a; --edge:#ffffff2e; --chrome:#161a2c;
  }

  /* 青竹色 → 卯の花色. The first of these that is not warm, which mostly
     shows up in how little else had to change: the mechanism is the same
     linear ramp with washes over it, and only the numbers moved.
     
     Bamboo is the constraint at 0.40 luminance — black clears it at 9:1 but
     the muted tier has to sit lower than the pale themes allow, closer to
     `--fg` than it would like. */
  :root[data-theme="bamboo"], .t-bamboo {
    color-scheme:light;
    --bg:#6fb98f; --page:var(--bg);
    --stops:#6fb98f 0%,#b4d8c2 48%,#f7f7f7 100%;
    --surface:#00000014; --raised:#00000021; --line:#00000038;
    --fg:hsl(150 45% 8%); --dim:hsl(150 32% 17%); --faint:hsl(150 26% 21%);
    --accent:hsl(155 55% 22%); --on-accent:#fff;
    --warn:hsl(20 88% 16%); --warn-bg:#00000014; --ok:hsl(190 62% 14%);
    --shadow:0 1px 3px hsl(150 40% 15% / .22);
    --field:#0000001a; --edge:#00000029; --chrome:#dfeee6;
  }

  /* 珊瑚絹 → 象牙色. Both stops are pale — luminance 0.62 to 0.83 — which
     makes this the least demanding of them: black runs 13.5:1 to 17.6:1 and
     the muted tier has room to be genuinely muted rather than nearly black,
     which is the compromise every darker background here has forced.
     
     Nearest neighbour is the dawn theme, and the difference is the point of
     it: dawn opens on a saturated salmon and has somewhere to fall to. This
     one starts pale and stays there. */
  :root[data-theme="silk"], .t-silk {
    color-scheme:light;
    --bg:#eac8b8; --page:var(--bg);
    --stops:#eac8b8 0%,#eddbcd 46%,#f3ead7 100%;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(20 40% 11%); --dim:hsl(20 30% 26%); --faint:hsl(20 25% 32%);
    --accent:hsl(14 55% 30%); --on-accent:#fff;
    --warn:hsl(350 88% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(20 40% 25% / .18);
    --field:#00000017; --edge:#00000024; --chrome:#f0e2d6;
  }

  /* A warm halo over white — the shadcn neutral palette with the light
     switched on above it.
     
     The glow is placed on the wash layer, which overhangs the screen by 380px,
     so `at 48% 4%` puts its centre well above the top edge and only the lower
     falloff is ever seen. That is what makes it read as light coming into the
     page rather than as a circle drawn on it.
     
     Contrast is a non-issue for once: cream on white, nothing below 15:1, so
     the neutrals can be exactly the ones the component ships with. */
  :root[data-theme="haze"], .t-haze {
    color-scheme:light;
    --bg:#fff; --page:var(--bg);
    --stops:rgb(236,226,208) 0%,#fff 70%;
    --gradient-default:circular;
    --surface:hsl(0 0% 96.1%); --raised:hsl(0 0% 93%); --line:hsl(0 0% 89.8%);
    --fg:hsl(0 0% 3.9%); --dim:hsl(0 0% 38%); --faint:hsl(0 0% 46%);
    --accent:hsl(0 0% 9%); --on-accent:hsl(0 0% 98%);
    --warn:hsl(20 88% 36%); --warn-bg:hsl(38 80% 94%); --ok:hsl(150 62% 28%);
    --shadow:0 1px 2px hsl(0 0% 0% / .08);
    --field:hsl(0 0% 96.1%); --edge:hsl(0 0% 91%); --chrome:hsl(0 0% 98%);
  }

  /* #4D80E6 is hsl(220 75% 60%), where white measures 3.79:1 — enough for a
     heading, not for a conversation. So the colour is the page, and every
     surface that carries text is a *deeper* step of the same blue: bubbles at
     5.1:1, your own at 10.7:1. Differentiation and legibility from one move. */
  :root[data-theme="indigo"], .t-indigo {
    color-scheme:dark;
    --bg:#4d80e6; --page:var(--bg); --stops:var(--no-stops);
    --surface:hsl(220 72% 52%); --raised:hsl(220 72% 46%); --line:hsl(220 60% 68%);
    --fg:#fff; --dim:hsl(220 70% 90%); --faint:hsl(220 45% 80%);
    --accent:hsl(220 72% 30%); --on-accent:#fff;
    --warn:hsl(40 95% 88%); --warn-bg:hsl(220 72% 38%); --ok:hsl(150 65% 88%);
    --shadow:0 1px 3px hsl(220 60% 18% / .3);
    --field:hsl(220 72% 42%); --edge:hsl(220 60% 44%); --chrome:hsl(220 72% 47%);
    --busy:hsl(196 100% 72%);
  }
  /* Motion, as three durations and two curves. Naming them is the whole
     discipline: every transition below picks from this list, so nothing drifts
     into feeling different for no reason. */
  :root {
    --mono:"IBM Plex Mono",ui-monospace,SFMono-Regular,"SF Mono",Menlo,monospace;
    /* A palette supplies `--stops`; this decides the shape they are drawn in,
       so form and colour are independent and any theme can be seen three
       ways. A palette with no stops collapses to transparent and its `--bg`
       shows through unchanged.
       
       The overhang is tied to the blur rather than fixed: at 100px the layer
       needs room for the blur to pull from, and at 0px it must be exactly the
       viewport or the outer stops fall off the screen. */
    --gradient-blur:100px;
    --page:var(--bg);
    --no-stops:transparent 0%,transparent 100%;
    --stops:var(--no-stops);
    --wash:linear-gradient(180deg,var(--stops,var(--no-stops)));
    --dur-fast:.12s; --dur:.22s; --dur-slow:.34s;
    --ease:cubic-bezier(.32,.72,0,1);          /* quick out, gentle in */
    --ease-spring:cubic-bezier(.34,1.35,.64,1); /* the same, with a nudge past */
  }
  /* Motion is decoration. Anyone who has asked their system to stop it gets
     the end state immediately, not a faster version of the journey. */
  @media (prefers-reduced-motion: reduce) {
    *, *::before, *::after {
      animation-duration:.01ms !important; animation-iteration-count:1 !important;
      transition-duration:.01ms !important;
    }
  }

  * { box-sizing:border-box; -webkit-tap-highlight-color:transparent; }
  /* Selection and focus in the theme's own colours: the browser defaults are
     the one place a themed page reverts to somebody else's blue. */
  ::selection { background:var(--accent); color:var(--on-accent); }
  :focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
  /* A scrollbar the width of a hairline, in the palette. Desktop only —
     touch has none — but this is a page that gets opened on a laptop too. */
  #log::-webkit-scrollbar, .tree::-webkit-scrollbar, .ask .body::-webkit-scrollbar {
    width:5px;
  }
  #log::-webkit-scrollbar-thumb, .tree::-webkit-scrollbar-thumb,
  .ask .body::-webkit-scrollbar-thumb {
    background:var(--line); border-radius:3px;
  }
  :root[data-gradient="circular"] {
    --wash:radial-gradient(125% 92% at 50% 0%,var(--stops,var(--no-stops)));
  }
  :root[data-gradient="angular"] {
    --wash:conic-gradient(from 195deg at 48% 46%,var(--stops,var(--no-stops)));
  }

  /* The gradient lives on its own layer so it can be blurred without blurring
     the conversation with it. Fixed and static, so it composites once and the
     log scrolls over it. A theme may lay a veil over the top — see dusk, where
     the colours are too mid-toned to take text without one. */
  body::before {
    content:""; position:fixed; z-index:-1; pointer-events:none;
    inset:calc(-1.8 * var(--gradient-blur));
    background:linear-gradient(var(--veil,transparent),var(--veil,transparent)), var(--wash);
    filter:blur(var(--gradient-blur));
  }

  /* The page is painted here and nowhere else. Everything structural above it
     is transparent, so a gradient stays one continuous wash from the status
     bar to the composer instead of each bar restating it inside its own box.
     It also means the strip beyond the body on iOS is the page, not a guess
     at which surface ought to stand in for it. */
  html { height:100%; overflow:hidden; background:var(--page); }
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
    margin:0; background:transparent; color:var(--fg);
    display:flex; flex-direction:column;
    /* Monospace throughout. It is what the thing being read *is* — terminal
       output, paths, commands — and a proportional face spent every line
       pretending otherwise. Slightly smaller and slightly tighter than the
       sans it replaces, because mono runs wide. */
    font:11.5px/1.42 var(--mono);
    letter-spacing:-.035em;
    -webkit-font-smoothing:antialiased;
  }
  button, a { font:inherit; color:inherit; }
  /* Everything tappable answers the finger. Without this the page is correct
     and feels dead: on a touch screen the press *is* the feedback, because
     there is no cursor and no hover to tell you the thing is live. */
  .icon, .act, .proj, .agent, .server, .new button, .theme button,
  .ask button, .sheet button, .chip button, #stale button {
    transition: transform var(--dur-fast) var(--ease),
                background-color var(--dur-fast) var(--ease),
                opacity var(--dur-fast) var(--ease);
  }
  .icon:active, .act:active, .new button:active, .theme button:active,
  .ask button:active, .sheet button:active, #stale button:active {
    transform:scale(.94);
  }
  /* Full-width rows scale less: the same 6% reads as a lurch across 340px. */
  .proj:active, .agent:active, .server:active { transform:scale(.985); }

  /* ---- header ---------------------------------------------------------- */
  header {
    flex:none; display:flex; align-items:center; gap:11px;
    padding:max(9px,env(safe-area-inset-top)) 10px 9px 14px;
    /* The page itself, divided by a rule rather than a shade. A header in its
       own tone is a second surface for no reason: nothing sits *on* it. */
    background:transparent; border-bottom:1px solid var(--line);
  }
  .who { display:flex; flex-direction:column; min-width:0; flex:1; gap:1px; }
  .who b { font-size:13px; font-weight:500; letter-spacing:-.045em; }
  .who span {
    font-size:9.5px; color:var(--dim);
    overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  }
  .dot { width:9px; height:9px; border-radius:50%; background:var(--faint); flex:none; }
  /* Alert colours are chosen per theme to differ from the page in hue, not
     just in lightness — an amber warning on an amber page is the page. As
     fills for a dot or a badge they answer to the 3:1 rule for graphics; the
     one theme that cannot clear 4.5 either way (indigo, a saturated mid-blue)
     uses them only that way. */
  .dot.blocked { background:var(--warn); box-shadow:0 0 0 3px color-mix(in srgb,var(--warn) 18%,transparent); }
  /* The accent doubles as the busy dot, which works until the accent *is* a
     shade of the background — then it disappears into it. Themes that need a
     different colour say so. */
  .dot.working { background:var(--busy,var(--accent)); animation:pulse 1.5s ease-in-out infinite; }
  .dot.idle { background:var(--ok); }
  @keyframes pulse { 50% { opacity:.3; } }

  .icon {
    flex:none; width:38px; height:38px; border-radius:9px; position:relative;
    display:grid; place-items:center; font-size:14px;
    background:none; border:1px solid transparent;
  }
  .icon:active { background:var(--surface); }
  .icon .badge {
    position:absolute; top:2px; right:1px; min-width:16px; height:16px; padding:0 4px;
    background:var(--warn); color:#1a1305; border-radius:999px;
    font-size:8px; font-weight:500; line-height:16px; text-align:center;
  }

  /* ---- conversation ---------------------------------------------------- */
  #log { flex:1; overflow-y:auto; -webkit-overflow-scrolling:touch; padding:12px 14px 6px; }
  /* A flex row per message, so a bubble hugs its text but a wide code block
     inside one cannot shrink it to a column of single letters. */
  /* Without boxes, the space between turns is what separates them. */
  .row { display:flex; margin-bottom:11px; }
  .row.you { justify-content:flex-end; }
  /* Only rows that are new carry `fresh`. The log is rebuilt whole on every
     change, so animating all of it would re-play the entire conversation each
     time a single line arrives. */
  .row.fresh, .tool.fresh { animation:rise var(--dur) var(--ease) both; }
  @keyframes rise {
    from { opacity:0; transform:translateY(7px) scale(.985); }
  }
  /* No bubbles by default: the text sits on the page, and alignment plus a
     heavier weight say who spoke — cues that survive every theme, where a
     colour would not. Turned on, your own turns take the theme's accent at
     80%, so the page still shows through and the bubble belongs to the
     palette rather than sitting on top of it. */
  :root[data-bubbles="on"] .row.you .msg {
    background:color-mix(in srgb, var(--accent) 80%, transparent);
    color:var(--on-accent); padding:7px 11px;
    border-radius:12px 12px 3px 12px;
  }
  :root[data-bubbles="on"] .row.you .msg code { background:#ffffff2e; }
  .msg {
    max-width:88%; min-width:0; white-space:pre-wrap; overflow-wrap:anywhere;
  }
  .row.you { padding-left:12%; }
  .row.you .msg { font-weight:500; }
  .row.pending .msg { opacity:.5; }
  /* Already monospace, so inline code is marked by weight and a wash rather
     than by changing face — which now would not read as anything. */
  .msg code { background:#8b93a426; border-radius:4px; padding:1px 4px; font-weight:500; }
  /* On `--surface` rather than `--field`: with the bubbles gone this sits
     directly on the page, and in some themes a field *is* the page colour —
     it was only ever visible because a bubble was behind it. */
  .msg pre {
    margin:8px 0 4px; padding:9px 11px; border-radius:8px;
    background:var(--surface); border:1px solid var(--edge);
    max-width:100%; overflow-x:auto; -webkit-overflow-scrolling:touch;
  }
  /* Code keeps its natural advance: tracking is what makes a monospace
     column line up, and this is the one place that matters. */
  .msg pre code { background:none; padding:0; font-weight:400; font-size:10.5px;
                  letter-spacing:normal; }
  .row.you .msg pre { background:#0000002e; }

  /* A tool call is not speech: one dim line, so the conversation does not
     look like it skipped a beat. */
  .tool {
    display:flex; align-items:baseline; gap:7px; margin:0 0 9px; color:var(--dim);
    font-size:10.5px; min-width:0;
  }
  .tool .n { font-weight:500; flex:none; }
  .tool .d {
    font-size:9.5px; color:var(--faint);
    overflow:hidden; text-overflow:ellipsis; white-space:nowrap; min-width:0;
  }
  .when {
    display:flex; align-items:center; gap:10px; margin:12px 2px 10px;
    color:var(--faint); font-size:8.5px; letter-spacing:.06em;
  }
  .when::before, .when::after { content:""; flex:1; height:1px; background:var(--line); opacity:.55; }
  .raw {
    font-size:9px; line-height:1.3; color:var(--dim); letter-spacing:normal;
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
  .empty { color:var(--dim); text-align:center; padding:48px 20px; font-size:12px; }
  /* Shaped like the conversation that is about to replace it, so the first
     paint is not a blank page with a word in the middle. */
  .skeleton { padding:4px 0; }
  .skeleton div {
    height:44px; border-radius:19px; margin-bottom:9px; background:var(--surface);
    animation:breathe 1.6s var(--ease) infinite;
  }
  .skeleton div:nth-child(1) { width:62%; }
  .skeleton div:nth-child(2) { width:80%; animation-delay:.15s; }
  .skeleton div:nth-child(3) { width:45%; margin-left:auto; animation-delay:.3s;
                               background:var(--accent); opacity:.5; }
  @keyframes breathe { 50% { opacity:.45; } }

  /* ---- the question an agent is stopped on ----------------------------- */
  .ask {
    flex:none; margin:0 10px 8px; padding:12px 13px 11px;
    background:var(--warn-bg); border:1px solid var(--warn); border-radius:12px;
  }
  .ask h3 {
    margin:0 0 8px; font-size:9px; font-weight:500; letter-spacing:.12em;
    text-transform:uppercase; color:var(--warn);
  }
  .ask .body {
    font:10px/1.4 ui-monospace,SFMono-Regular,Menlo,monospace; color:var(--fg);
    white-space:pre-wrap; overflow-wrap:anywhere; max-height:33vh; overflow-y:auto;
    margin-bottom:11px;
  }
  .ask button {
    display:block; width:100%; text-align:left; margin-top:7px; padding:11px 13px;
    border-radius:9px; border:1px solid var(--line); background:var(--surface);
    font-size:12px; line-height:1.25;
  }
  .ask button.first { background:var(--accent); border-color:var(--accent); color:var(--on-accent); font-weight:500; }
  .ask button:active { transform:scale(.985); }
  .ask .key { opacity:.55; font-weight:500; margin-right:7px; }

  /* ---- composer -------------------------------------------------------- */
  .composer {
    flex:none; display:flex; gap:8px; align-items:flex-end;
    padding:8px 10px calc(8px + env(safe-area-inset-bottom));
    background:transparent; border-top:1px solid var(--line);
  }
  textarea {
    flex:1; min-width:0; resize:none; max-height:132px;
    padding:9px 13px; border-radius:14px; border:1px solid var(--line);
    background:var(--field); color:var(--fg);
    /* 16px keeps iOS from zooming the page when the field takes focus. */
    font-family:var(--mono); font-size:16px; line-height:1.3; letter-spacing:-.03em;
  }
  textarea:focus { outline:none; border-color:var(--accent); }
  textarea::placeholder { color:var(--faint); }
  .act {
    flex:none; width:40px; height:40px; border-radius:50%; border:1px solid var(--line);
    background:var(--field); display:grid; place-items:center; font-size:14px;
  }
  .act.send { background:var(--accent); border-color:var(--accent); color:var(--on-accent); font-size:17px; }
  .act.send:disabled { opacity:.3; transform:scale(.92); }
  .act.on { background:var(--warn); border-color:var(--warn); color:#1a1305; }
  .note {
    flex:none; color:var(--dim); font-size:10.5px; text-align:center;
    padding:0 14px 7px;
  }

  /* What the + offers. A sheet rather than going straight to the picker,
     because queueing work was what this button did and losing it silently
     would be worse than one extra tap. */
  /* Animated with max-height rather than `hidden`, because display:none has
     no in-between for a transition to happen in. */
  .sheet {
    flex:none; display:flex; flex-direction:column; gap:6px;
    padding:0 10px; max-height:0; opacity:0; overflow:hidden;
    transform:translateY(6px);
    transition: max-height var(--dur) var(--ease), opacity var(--dur) var(--ease),
                transform var(--dur) var(--ease-spring), padding var(--dur) var(--ease);
  }
  .sheet.open { max-height:220px; opacity:1; transform:none; padding:0 10px 8px; }
  .sheet button {
    padding:12px; border-radius:10px; font-size:11.5px; font-weight:500;
    border:1px solid var(--line); background:var(--surface); color:var(--fg);
  }
  .sheet button.cancel { font-weight:400; color:var(--dim); }

  #attached { flex:none; display:flex; flex-wrap:wrap; gap:6px; padding:0 10px; }
  #attached:not(:empty) { padding-bottom:8px; }
  .chip {
    display:flex; align-items:center; gap:7px; max-width:100%;
    padding:6px 8px 6px 10px; border-radius:8px;
    background:var(--surface); border:1px solid var(--line); font-size:10.5px;
  }
  .chip { animation:rise var(--dur) var(--ease-spring) both; }
  .chip img { width:26px; height:26px; border-radius:5px; object-fit:cover; }
  .chip span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .chip button { border:0; background:none; color:var(--dim); font-size:13px; padding:0 2px; }

  /* The palette panel: everything about how the page looks, in one place,
     one tap from the header rather than buried in the projects drawer. */
  .palette {
    position:fixed; left:0; right:0; bottom:0; z-index:5;
    transform:translateY(101%); transition:transform var(--dur-slow) var(--ease);
    background:var(--chrome); border-top:1px solid var(--line);
    border-radius:16px 16px 0 0;
    padding:4px 16px calc(18px + env(safe-area-inset-bottom));
    max-height:76vh; overflow-y:auto;
  }
  .palette.open { transform:none; }
  .palette h2 {
    font-size:8px; letter-spacing:.18em; text-transform:uppercase;
    color:var(--faint); margin:16px 0 8px; font-weight:500;
  }
  .palette h2 span { letter-spacing:0; text-transform:none; }
  .forms { display:flex; gap:6px; }
  .forms button {
    flex:1; padding:9px 0; border-radius:9px; font-size:10.5px; font-weight:500;
    border:1px solid var(--line); background:none; color:var(--dim);
  }
  .forms button.on { background:var(--raised); color:var(--fg); border-color:var(--fg); }
  .palette input[type=range] {
    width:100%; margin:2px 0 0; accent-color:var(--accent); background:none;
  }

  /* ---- drawer ---------------------------------------------------------- */
  .scrim {
    position:fixed; inset:0; background:#00000073; opacity:0; pointer-events:none;
    transition:opacity var(--dur) var(--ease); backdrop-filter:blur(1px);
  }
  .scrim.open { opacity:1; pointer-events:auto; }
  aside {
    position:fixed; top:0; right:0; bottom:0; width:min(87vw,352px);
    background:var(--chrome); border-left:1px solid var(--line);
    transform:translateX(100%); transition:transform var(--dur-slow) var(--ease);
    display:flex; flex-direction:column; padding-top:env(safe-area-inset-top);
  }
  aside.open { transform:none; }
  aside h2 {
    font-size:8px; letter-spacing:.18em; text-transform:uppercase; color:var(--faint);
    margin:16px 18px 7px; font-weight:500;
  }
  .tree { flex:1; overflow-y:auto; padding-bottom:8px; }
  .proj, .agent {
    display:flex; align-items:center; gap:10px; width:100%; text-align:left;
    background:none; border:0; padding:12px 16px;
  }
  .proj { font-weight:500; }
  .proj .caret { color:var(--faint); font-size:8px; width:10px; }
  .proj .name { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .proj .n { margin-left:auto; color:var(--faint); font-size:10px; font-weight:400; }
  .agent { padding:10px 16px 10px 38px; }
  .agent .label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .agent.current { background:var(--raised); }
  .agent .what { color:var(--dim); font-size:10px; margin-left:auto; flex:none; }
  .pill {
    background:var(--warn); color:#1a1305; border-radius:999px;
    font-size:8px; font-weight:500; padding:1px 6px;
  }
  .server {
    display:flex; align-items:center; gap:9px; width:100%; text-align:left;
    padding:8px 16px 8px 38px; color:var(--fg); font-size:12px;
    text-decoration:none;
  }
  .server .port {
    font-size:10px; line-height:1; color:var(--accent);
    font-weight:500; flex:none;
  }
  .server .cmd { color:var(--dim); font-size:10px; margin-left:auto; flex:none; }
  .new { display:flex; gap:8px; padding:2px 16px 12px 38px; }
  .new button {
    flex:1; font-size:11px; padding:8px; border-radius:10px;
    border:1px dashed var(--line); background:none; color:var(--dim);
  }
  .notify {
    flex:none; margin:8px 16px 0; padding:11px 13px; text-align:left;
    border:1px solid var(--line); border-radius:11px; background:var(--bg);
    color:var(--fg); font-size:11.5px; line-height:1.3;
  }
  .notify.on { border-color:var(--ok); color:var(--dim); }
  /* Five themes will not fit a segmented control, so each is a swatch of the
     colour it actually is — which says more than its name does anyway. */
  .theme { display:flex; flex-wrap:wrap; gap:6px; }
  .theme button {
    display:flex; align-items:center; gap:7px; padding:7px 11px 7px 8px;
    border:1px solid var(--line); border-radius:999px; background:none;
    color:var(--dim); font-size:10.5px; font-weight:500;
  }
  /* Painted from the palette the swatch is named for — `--wash` over
     `--page`, the same two layers the page itself is built from. A swatch
     cannot fall out of step with its theme when it *is* the theme. */
  .theme button i {
    width:15px; height:15px; border-radius:50%; flex:none;
    border:1px solid #0000002e;
    background:linear-gradient(155deg,var(--stops,var(--no-stops))), var(--page);
    background-size:cover;
  }
  .t-system { --page:linear-gradient(105deg,#f2f3f7 50%,#0c0e13 50%);
               --stops:var(--no-stops); }
  /* Dark is the base palette rather than an override, so it has no block of
     its own to borrow; its swatch needs the one value it would have taken. */
  .t-dark { --bg:#0c0e13; --page:var(--bg); --stops:var(--no-stops); }
  .theme button.on { background:var(--raised); color:var(--fg); border-color:var(--fg); }

  #stale {
    flex:none; margin:0 10px 8px; padding:11px 13px; border-radius:14px;
    background:var(--warn-bg); border:1px solid var(--warn);
    font-size:11px; line-height:1.3;
  }
  #stale b { display:block; color:var(--warn); margin-bottom:3px; font-weight:500; }
  #stale button {
    margin-top:8px; padding:7px 11px; border-radius:9px; font-size:11px;
    border:1px solid var(--line); background:var(--surface); color:var(--fg);
  }

  /* ?debug=1 — what the browser thinks the viewport is. */
  .debug {
    position:fixed; left:8px; bottom:8px; z-index:99; margin:0;
    background:#000000d9; color:#7CFF9B; border-radius:8px; padding:7px 9px;
    font-size:8px; line-height:1.3; white-space:pre;
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
  <button class="icon" onclick="togglePalette()" title="theme">◑</button>
  <button class="icon" onclick="toggleDrawer()" title="projects">☰<span class="badge" id="hbadge" hidden></span></button>
</header>

<div id="log"><div class="skeleton"><div></div><div></div><div></div></div></div>
<div id="stale" hidden></div>
<div id="ask"></div>
<div class="note" id="note" hidden></div>

<div class="sheet" id="sheet">
  <button onclick="pickFile()">Photo or file</button>
  <button onclick="hideSheet(); queueMessage()">Queue as a TODO</button>
  <button class="cancel" onclick="hideSheet()">Cancel</button>
</div>
<div id="attached"></div>

<div class="composer">
  <input type="file" id="file" accept="image/*,text/*,.pdf,.log,.json" multiple hidden>
  <button class="act" id="queue" onclick="toggleSheet()" title="attach or queue">＋</button>
  <textarea id="msg" rows="1" placeholder="Message"></textarea>
  <button class="act" id="mic" onclick="toggleMic()" title="dictate">🎤</button>
  <button class="act send" id="send" onclick="sendMessage()" disabled>↑</button>
</div>

<div class="scrim" id="paletteScrim" onclick="togglePalette()"></div>
<section class="palette" id="palette">
  <h2>theme</h2>
  <div class="theme" id="theme"></div>
  <h2>gradient</h2>
  <div class="forms" id="forms"></div>
  <h2>your messages</h2>
  <div class="forms" id="bubbles">
    <button onclick="setBubbles('off')" data-bubbles="off">Plain</button>
    <button onclick="setBubbles('on')" data-bubbles="on">Bubble</button>
  </div>
  <h2>blur <span id="blurValue"></span></h2>
  <input type="range" id="blur" min="0" max="240" step="10" oninput="setBlur(this.value)">
</section>

<div class="scrim" id="scrim" onclick="toggleDrawer()"></div>
<aside id="drawer">
  <h2>projects</h2>
  <div class="tree" id="tree"></div>
  <button class="notify" id="notify" onclick="enablePush()">
    <span>Notify me when an agent is blocked</span>
  </button>
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
  attached = [];
  renderAttached();
  hideSheet();
  thread = [];
  drawn = 0;
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
  const typed = take();
  // An attachment on its own is a message: "look at this" is implied.
  if (!typed && !attached.length) return;
  const text = [typed, ...attached.map(a => a.path)].filter(Boolean).join("\n");
  attached = [];
  renderAttached();
  sent.push(text);                      // appears immediately, like a chat app
  render();
  post("/api/reply", { agent: current, text });
}

/* ---- attachments ------------------------------------------------------- */

/* Files land on the desktop and the agent is handed the path, which is how
   both Claude and Codex take an image. Nothing is sent until you do, so a
   photo can have a caption. */
let attached = [];

function toggleSheet() {
  document.getElementById("sheet").classList.toggle("open");
}

function hideSheet() { document.getElementById("sheet").classList.remove("open"); }

function pickFile() {
  hideSheet();
  document.getElementById("file").click();
}

document.getElementById("file").addEventListener("change", async event => {
  const files = [...event.target.files];
  event.target.value = "";               // so the same photo can be picked twice
  for (const file of files) {
    note("sending " + file.name + "…");
    try {
      const res = await fetch(
        q("/api/upload?agent=" + encodeURIComponent(current) +
          "&name=" + encodeURIComponent(file.name)),
        { method: "POST", body: file });
      if (!res.ok) throw new Error(await res.text());
      const { path } = await res.json();
      attached.push({
        path,
        name: file.name,
        preview: file.type.startsWith("image/") ? URL.createObjectURL(file) : null,
      });
      note("");
      renderAttached();
    } catch (err) {
      note("could not send " + file.name + ": " + err.message);
    }
  }
});

function removeAttached(index) {
  const [gone] = attached.splice(index, 1);
  if (gone?.preview) URL.revokeObjectURL(gone.preview);
  renderAttached();
}

function renderAttached() {
  document.getElementById("attached").innerHTML = attached.map((a, i) => `
    <span class="chip">
      ${a.preview ? '<img src="' + a.preview + '" alt="">' : ""}
      <span>${esc(a.name)}</span>
      <button onclick="removeAttached(${i})" aria-label="remove">×</button>
    </span>`).join("");
  document.getElementById("send").disabled =
    !attached.length && !document.getElementById("msg").value.trim();
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
  document.getElementById("send").disabled = !e.target.value.trim() && !attached.length;
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

/* iOS copies `apple-mobile-web-app-*` into the web clip when you add the app
   to the home screen, and never reads them again. An icon added before those
   metas changed still runs under the old ones.

   That matters here because under `black-translucent` iOS hands out a web
   view the height of the screen *minus* the status bar, pinned to the top —
   so the last status-bar-height of the screen sits outside the page, and no
   amount of CSS can reach it. It is the gap under the composer, and only
   re-adding the icon clears it.

   The signature is unambiguous: the page is short by exactly the inset it is
   being told overlaps its top. Worth saying out loud, because from the inside
   it looks like a layout bug and no amount of fixing the layout helps. */
function checkStaleWebClip() {
  const banner = document.getElementById("stale");
  if (navigator.standalone !== true || store.get("staleDismissed", "") === "1") {
    banner.hidden = true;
    return;
  }
  const probe = document.createElement("div");
  probe.style.cssText = "position:fixed;visibility:hidden;padding-top:env(safe-area-inset-top)";
  document.body.appendChild(probe);
  const insetTop = parseFloat(getComputedStyle(probe).paddingTop) || 0;
  probe.remove();

  const missing = screen.height - innerHeight;
  const stale = insetTop > 0 && Math.abs(missing - insetTop) < 2;
  banner.hidden = !stale;
  if (stale && !banner.innerHTML) {
    banner.innerHTML =
      "<b>Re-add this app to your home screen</b>" +
      "iOS kept the old settings from when this icon was added, which leaves " +
      Math.round(missing) + "px of dead space below. Remove the icon, open the " +
      "link in Safari, and Share → Add to Home Screen." +
      '<button onclick="dismissStale()">Dismiss</button>';
  }
}

function dismissStale() {
  store.set("staleDismissed", "1");
  document.getElementById("stale").hidden = true;
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

/* Adding a theme is a palette in the stylesheet and a line here. The swatch
   draws itself from the palette, so there is no third place to keep in step. */
const THEMES = [
  ["system", "Auto"], ["light", "Light"], ["dark", "Dark"],
  ["gamboge", "Gamboge"], ["dawn", "Dawn"], ["milk", "Milk"], ["dusk", "Dusk"],
  ["haze", "Haze"], ["silk", "Silk"], ["bamboo", "Bamboo"],
  ["cinnabar", "Cinnabar"], ["indigo", "Indigo"],
];

document.getElementById("theme").innerHTML = THEMES.map(([name, label]) =>
  `<button onclick="setTheme('${name}')" data-theme-name="${name}">` +
  `<i class="t-${name}"></i>${label}</button>`).join("");

/* "system" follows the phone; the other two override it until you say
   otherwise. The status bar is told separately: on iOS it is chrome, not
   page, and only `theme-color` reaches it. */
function setTheme(mode) {
  if (mode === "system") localStorage.removeItem("theme");
  else localStorage.setItem("theme", mode);

  if (mode === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", mode);

  // The status bar is chrome, not page: only `theme-color` reaches it, and it
  // has to be told again whenever the palette moves under it.
  const top = getComputedStyle(document.documentElement).getPropertyValue("--bg").trim();
  document.querySelector('meta[name="theme-color"]').setAttribute("content", top);

  document.querySelectorAll("#theme button").forEach(button => {
    button.classList.toggle("on", button.dataset.themeName === mode);
  });

  // A palette may have been designed around one shape — dusk is a sweep, haze
  // is a glow. Offer it the first time that theme is picked; after that the
  // choice on screen is yours.
  const wants = getComputedStyle(document.documentElement)
    .getPropertyValue("--gradient-default").trim();
  if (wants && !store.get("gradient", "")) setForm(wants);
}

/* Which shape the stops are drawn in, and how far it is blurred. Both are
   about the page rather than about a theme, so they persist across themes —
   picking a new palette should not silently undo how you like to see it. */
const FORMS = [["linear", "Linear"], ["circular", "Circular"], ["angular", "Angular"]];

document.getElementById("forms").innerHTML = FORMS.map(([name, label]) =>
  `<button onclick="setForm('${name}')" data-form="${name}">${label}</button>`).join("");

function setForm(name) {
  store.set("gradient", name);
  document.documentElement.setAttribute("data-gradient", name);
  document.querySelectorAll("#forms button").forEach(button =>
    button.classList.toggle("on", button.dataset.form === name));
}

function setBubbles(mode) {
  store.set("bubbles", mode);
  document.documentElement.setAttribute("data-bubbles", mode);
  document.querySelectorAll("#bubbles button").forEach(button =>
    button.classList.toggle("on", button.dataset.bubbles === mode));
}

function setBlur(px) {
  store.set("blur", px);
  document.documentElement.style.setProperty("--gradient-blur", px + "px");
  document.getElementById("blurValue").textContent = px + "px";
  document.getElementById("blur").value = px;
}

let paletteOpen = false;
function togglePalette() {
  paletteOpen = !paletteOpen;
  document.getElementById("palette").classList.toggle("open", paletteOpen);
  document.getElementById("paletteScrim").classList.toggle("open", paletteOpen);
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

/* How much of the conversation had already been drawn last time, so what is
   drawn now can tell arriving from re-rendering. */
let drawn = 0;

function messagesHtml(a) {
  const parts = [];
  let last = null;
  let index = 0;
  // A first load animates as a short cascade; after that only the new line
  // moves, and it does not wait its turn behind the ones already on screen.
  const cascade = drawn === 0;
  for (const m of thread) {
    const fresh = index++ >= drawn ? " fresh" : "";
    const delay = cascade && fresh
      ? ` style="animation-delay:${Math.min(index * 28, 340)}ms"` : "";
    // A gap means the conversation was picked up later; say when.
    const at = m.at ? new Date(m.at) : null;
    if (at && (!last || at - last > 10 * 60 * 1000)) parts.push('<div class="when">' + clock(m.at) + "</div>");
    if (at) last = at;

    if (m.role === "tool") {
      const [name, detail] = m.text.split(" · ");
      parts.push('<div class="tool' + fresh + '"' + delay + '><span class="n">' + esc(name) + "</span>" +
                 '<span class="d">' + esc(detail || "") + "</span></div>");
    } else {
      parts.push('<div class="row' + fresh + " " + (m.role === "you" ? "you" : "") + '"' + delay + '>' +
                 '<div class="msg">' + markdown(m.text) + "</div></div>");
    }
  }
  if (!thread.length && a.tail.length) {
    // No journal we can read: the terminal is all there is.
    parts.push('<div class="raw">' + esc(a.tail.join("\n")) + "</div>");
  }
  for (const t of sent) parts.push('<div class="row you pending fresh"><div class="msg">' + esc(t) + "</div></div>");
  if (a.status === "working") parts.push('<div class="typing"><i></i><i></i><i></i></div>');
  drawn = thread.length;
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

setForm(store.get("gradient", "linear"));
setBubbles(store.get("bubbles", "off"));
setBlur(store.get("blur", "100"));
setTheme(localStorage.getItem("theme") || "system");
markPush(store.get("push", "") === "on");
showDebug();
checkStaleWebClip();
if (window.visualViewport) {
  visualViewport.addEventListener("resize", () => { showDebug(); checkStaleWebClip(); });
}
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
