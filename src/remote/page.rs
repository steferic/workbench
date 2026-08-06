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
    --accent:#4361ee; --accent-2:hsl(228 18% 42%); --on-accent:#fff;
    --warn:hsl(20 88% 52%); --warn-bg:hsl(20 40% 13%); --ok:hsl(150 62% 40%);
    --shadow:0 1px 2px #0000004d;
    /* Depth, in two pieces, shared by every palette.
       `--hairline` is a 1px ring rather than a border, and it takes its ink
       from `--fg`: that darkens a pale theme and lightens a dark one without
       either having to name a colour, and it is why nothing wearing this may
       also carry a border — the two together read as a doubled edge.
       `--lift` is the ring over four shadow layers whose offset and opacity
       both decay geometrically (×.54 a step, blur ≈1.33× the offset). That
       curve is the whole trick: one flat layer reads as a grey rectangle
       under the box, several stacked read as light falling away from it. The
       ink stays black, so on a dark theme the layers vanish and the ring
       carries the edge alone, which is all that is legible there anyway. */
    --hairline:0 0 0 1px color-mix(in srgb, var(--fg) 8%, transparent);
    --depth:
      0 4px 5.3px #0000000a,
      0 2.1px 2.9px #00000008,
      0 1.2px 1.6px #00000005,
      0 .6px .9px -1px #00000003;
    --lift:var(--depth), var(--hairline);
    /* The same curve, further off the page — for the one thing here that is
       genuinely floating rather than merely raised. `--depth` is scaled for a
       bubble sitting in the text; under a card held above the whole
       conversation it barely registers. Offsets ~3.5× and the ink about
       double, decay untouched. */
    --float:
      0 14px 18.7px #00000014,
      0 7.5px 10px #0000000f,
      0 4.2px 5.6px #0000000b,
      0 2.2px 3px -2px #00000008,
      0 .9px 1.2px #00000008;
    /* Selected, for the controls that used to say so with a `--fg` border:
       the same depth under a ring dark enough to read as a choice. */
    --lift-on:var(--depth), 0 0 0 1px color-mix(in srgb, var(--fg) 45%, transparent);
    /* Fields and code wells recede from the surface they sit on. Which
       direction that is depends on the theme, so it cannot be derived. */
    --field:#0c0e13; --edge:#2b313d; --chrome:#12151b;
  }
  @media (prefers-color-scheme: light) {
    :root:not([data-theme="dark"]) {
      color-scheme:light;
      --bg:#f2f3f7; --surface:#fff; --raised:#fff; --line:#e3e6ec;
      --fg:#14161b; --dim:#5f6779; --faint:#98a0b0;
      --accent:#3355e8; --accent-2:hsl(228 22% 62%); --on-accent:#fff;
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
    --accent:hsl(44 95% 22%); --accent-2:hsl(30 45% 52%); --on-accent:#fff;
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
    --accent:hsl(9 65% 32%); --accent-2:hsl(30 50% 62%); --on-accent:#fff;
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
    --accent:hsl(40 95% 24%); --accent-2:hsl(38 45% 58%); --on-accent:#fff;
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
    --accent:hsl(219 78% 22%); --accent-2:hsl(20 35% 62%); --on-accent:#fff;
    --warn:hsl(20 88% 14%); --warn-bg:#00000014; --ok:hsl(150 62% 12%);
    --shadow:0 1px 3px hsl(215 40% 15% / .25);
    --field:#0000001f; --edge:#00000030; --chrome:#c3c0c6;
  }

  /* 鳶色 / 薄紫鼠 / 退紅 / 生成色 — four stops, and the widest span of any of
     them: 0.09 luminance at the kite brown to 0.77 at the unbleached cream.
     
     Neither text colour survives that raw. Black fails on the brown at
     2.9:1, white fails on everything else — 1.3:1 on the cream. So this one
     needs the heaviest veil here, 42% white, which lifts the brown to 6.7:1
     for the body and leaves the muted tiers somewhere to sit (5.6 and 4.7).
     The light stops barely move under it — the cream goes 0.77 to 0.82 — so
     what the veil actually costs is the depth of the brown, and what it buys
     is the other three stops being usable at all. */
  :root[data-theme="kite"], .t-kite {
    color-scheme:light;
    --bg:#7d483e; --page:var(--bg);
    --stops:#7d483e 0%,#a098a8 38%,#ffb3a7 70%,#ece2d0 100%;
    --veil:#ffffff6b;
    --surface:#00000012; --raised:#0000001c; --line:#00000033;
    --fg:hsl(15 48% 8%); --dim:hsl(15 34% 15%); --faint:hsl(15 28% 20%);
    --accent:hsl(12 60% 26%); --accent-2:hsl(280 18% 46%); --on-accent:#fff;
    --warn:hsl(350 88% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(15 40% 22% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#e4d9d2;
  }

  /* 黄丹 → 水緑 → 青白磁 → 月白. Warm to cool across the whole page: a
     yellow-red that only touches the top, two greens that carry most of it,
     and a moonlit white at the foot. The span is wide enough that the mid
     tones do the work, so the ink is a near-black green rather than the
     orange's complement — matched to where the text actually sits. */
  :root[data-theme="porcelain"], .t-porcelain {
    color-scheme:light;
    --bg:#f05e1c; --page:var(--bg);
    --stops:#f05e1c 0%,#98b8b0 34%,#b8d5d3 66%,#f0f4f8 100%;
    --veil:#ffffff40;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(170 30% 8%); --dim:hsl(170 20% 17%); --faint:hsl(170 16% 26%);
    --accent:hsl(18 78% 32%); --accent-2:hsl(172 22% 38%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(170 30% 15% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#dce6e4;
  }

  /* 露草色 → 黄金色 → 薄水色 → 乳白色. The one palette here that turns back
     on itself: blue, then gold, then blue again, paler. Gold in the middle is
     what keeps the two blues from reading as one long fade. */
  :root[data-theme="dayflower"], .t-dayflower {
    color-scheme:light;
    --bg:#38a1db; --page:var(--bg);
    --stops:#38a1db 0%,#e2be86 36%,#bee6eb 70%,#f3f3f3 100%;
    --veil:#ffffff40;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(205 42% 10%); --dim:hsl(205 26% 19%); --faint:hsl(205 20% 28%);
    --accent:hsl(203 70% 28%); --accent-2:hsl(36 52% 34%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(205 40% 18% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#dde7ea;
  }

  /* 峰鼠 → 濃藍 → 二藍 → 鳥の子色. Three quarters of this palette is dark —
     charcoal, then indigo, then a purple — and only the last stop is warm.
     Left alone that is a dark theme, but a dark theme cannot take a cream
     bottom, which is where the composer sits. So the veil does the work
     instead: at 56% white the charcoal lifts to a mid grey that dark text
     clears comfortably, and what survives is the muted version of this
     palette rather than its full-strength one — which is what it looks like
     blurred anyway. */
  :root[data-theme="lacquer"], .t-lacquer {
    color-scheme:light;
    --bg:#282828; --page:var(--bg);
    --stops:#282828 0%,#283c58 32%,#614e6e 62%,#fff1cf 100%;
    --veil:#ffffff8f;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(260 20% 9%); --dim:hsl(260 14% 20%); --faint:hsl(260 12% 30%);
    --accent:hsl(258 26% 30%); --accent-2:hsl(215 40% 28%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(260 24% 15% / .22);
    --field:#00000017; --edge:#00000026; --chrome:#ece7dc;
  }

  /* 栗梅 → 樺茶色 → 萌黄春 → 月白. Two earths into a green so pale it reads
     as a grey, and then white — the widest jump in lightness of any palette
     here, which is why it needs a veil at all: the chestnut end is otherwise
     too deep for the ink the other three want. */
  :root[data-theme="chestnut"], .t-chestnut {
    color-scheme:light;
    --bg:#8b352d; --page:var(--bg);
    --stops:#8b352d 0%,#b4631d 34%,#d4dcc8 68%,#f0f4f8 100%;
    --veil:#ffffff73;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(10 40% 9%); --dim:hsl(10 30% 15%); --faint:hsl(10 24% 24%);
    --accent:hsl(8 54% 28%); --accent-2:hsl(28 72% 30%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(10 40% 18% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#e6e9e4;
  }

  /* 紫苑色 → 柑子色 → 白茶 → 薄卵色. Purple straight into mandarin, which is
     the sharpest turn in the set — and then it settles, twice, into two warm
     neutrals a shade apart. The orange is the loudest colour anywhere here,
     so the veil is light: dimming it would waste the palette. */
  :root[data-theme="aster"], .t-aster {
    color-scheme:light;
    --bg:#976e9a; --page:var(--bg);
    --stops:#976e9a 0%,#f08300 34%,#e6d3b1 68%,#fff5e8 100%;
    --veil:#ffffff45;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(285 28% 10%); --dim:hsl(285 18% 20%); --faint:hsl(285 14% 30%);
    --accent:hsl(288 28% 30%); --accent-2:hsl(32 82% 30%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(285 26% 18% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#efe6db;
  }

  /* 藍御納戸 → 浅葱色 → 露草色 → 桃色. The one palette that does not finish
     on a near-white: it ends on a peach, so the foot of the page carries as
     much colour as the head of it. Everything is mid-toned, which is the
     easier problem — one ink reads across the lot once the slate end is
     lifted. */
  :root[data-theme="asagi"], .t-asagi {
    color-scheme:light;
    --bg:#3d5a6c; --page:var(--bg);
    /* The peach lands at 86%, not 100%. The wash layer is inset by 1.8× the
       blur on every side, so the gradient is taller than the screen and its
       two ends fall outside it — a stop at 100% is never seen. It does not
       show on the palettes that finish on a near-white, where the last two
       stops are near enough the same. Here it cost the one colour that makes
       this palette what it is. */
    --stops:#3d5a6c 0%,#00a3a3 28%,#38a1db 58%,#f47983 86%;
    --veil:#ffffff6b;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(200 40% 10%); --dim:hsl(200 26% 20%); --faint:hsl(200 20% 30%);
    --accent:hsl(200 62% 24%); --accent-2:hsl(180 100% 20%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(200 40% 18% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#dfe6ea;
  }

  /* 薄墨色 → 退紅 → 表萌黄絹 → 桜色. The only palette here that needs no veil
     at all: nothing in it is darker than a mid grey, so the ink clears 7.2:1
     on the worst of the four untouched. Grey into pink into a sage and back
     to pink — it holds together because the two pinks bracket the green. */
  :root[data-theme="blossom"], .t-blossom {
    color-scheme:light;
    --bg:#a3a3a3; --page:var(--bg);
    --stops:#a3a3a3 0%,#f3a6b1 30%,#c8d0b8 60%,#fcc9b9 88%;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(350 18% 9%); --dim:hsl(350 12% 19%); --faint:hsl(350 10% 28%);
    --accent:hsl(350 44% 32%); --accent-2:hsl(95 18% 32%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(350 18% 18% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#eadedb;
  }

  /* 鉄色 → 空色 → 朝鼠 → 桜色. Iron straight into a sky, which is the largest
     single jump in the set — 0.035 to 0.44 in luminance between two adjacent
     stops. The 48% veil is set by the iron alone; everything after it was
     already light enough. */
  :root[data-theme="iron"], .t-iron {
    color-scheme:light;
    --bg:#2b3733; --page:var(--bg);
    --stops:#2b3733 0%,#7ec7d8 32%,#c0c0bc 62%,#fcc9b9 88%;
    --veil:#ffffff7a;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(190 26% 9%); --dim:hsl(190 20% 15%); --faint:hsl(190 16% 24%);
    --accent:hsl(190 46% 28%); --accent-2:hsl(15 48% 34%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(190 26% 16% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#e8dedb;
  }

  /* 茜色 → 照柿 → 柿秋 → 薄紅. Four reds and no relief anywhere: this is the
     only palette with nothing pale in it at all.

     Which settles which way it has to go. A white veil strong enough for dark
     text takes the madder to a dusty pink and throws the palette away — the
     whole point of it is the saturation. A black one keeps the reds red and
     turns it into a dark theme: 35% puts white at 6.4:1 on the worst stop
     against 3.1:1 untouched, and it stays recognisably 茜. Cinnabar's move,
     for the same reason. */
  :root[data-theme="akane"], .t-akane {
    color-scheme:dark;
    --bg:#b7282e; --page:var(--bg);
    --stops:#b7282e 0%,#d34e36 32%,#c87840 62%,#f2666c 88%;
    --veil:#00000059;
    --surface:#ffffff14; --raised:#ffffff21; --line:#ffffff3d;
    --fg:#fff; --dim:hsl(10 30% 90%); --faint:hsl(10 24% 84%);
    --accent:#b7282e; --accent-2:hsl(25 55% 40%); --on-accent:#fff;
    --warn:hsl(40 95% 72%); --warn-bg:#ffffff14; --ok:hsl(150 60% 68%);
    --busy:hsl(200 95% 72%);
    --shadow:0 1px 3px #00000066;
    --field:#ffffff1a; --edge:#ffffff2e; --chrome:#3a1416;
  }

  /* 洗柿 → 錆鼠緑 → 鴇羽色 → 黄色. Ends on a full yellow rather than a
     neutral, and needs no veil to do it — the washed persimmon it starts on
     is already light enough to take the ink. Two warm stops with a rust green
     wedged between them, which is what stops it reading as one long blush. */
  :root[data-theme="persimmon"], .t-persimmon {
    color-scheme:light;
    --bg:#d3826e; --page:var(--bg);
    --stops:#d3826e 0%,#88a8a0 30%,#f58f84 60%,#ffd700 88%;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(15 30% 9%); --dim:hsl(15 20% 18%); --faint:hsl(15 16% 27%);
    --accent:hsl(12 52% 32%); --accent-2:hsl(165 20% 30%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(15 30% 18% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#f0e4c4;
  }

  /* 夕鼠 → 縹空 → 白. Three stops and no hue in the first or last — a grey
     falling through one blue into white. The quietest thing here. */
  :root[data-theme="twilight"], .t-twilight {
    color-scheme:light;
    --bg:#606060; --page:var(--bg);
    --stops:#606060 0%,#7898b8 46%,#f2f2f0 88%;
    --veil:#ffffff57;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(215 20% 9%); --dim:hsl(215 12% 19%); --faint:hsl(215 10% 28%);
    --accent:hsl(215 42% 32%); --accent-2:hsl(215 10% 34%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(215 20% 18% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#e9e9e7;
  }

  /* 緑青色 → 薄水色 → 白練. One hue, three lightnesses — the verdigris of
     weathered copper draining to the white of unbleached silk. A 12% veil is
     the lightest here; only the first stop needed anything. */
  :root[data-theme="verdigris"], .t-verdigris {
    color-scheme:light;
    --bg:#48929b; --page:var(--bg);
    --stops:#48929b 0%,#bee6eb 46%,#fdfbf6 88%;
    --veil:#ffffff1f;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(188 32% 9%); --dim:hsl(188 26% 15%); --faint:hsl(188 20% 24%);
    --accent:hsl(188 50% 26%); --accent-2:hsl(188 24% 40%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(188 32% 16% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#e4eeef;
  }

  /* 夜木鼠 → 海松緑 → 鳥の子色. Two near-blacks a hue apart — one brown, one
     green, close enough that the change between them is barely a change —
     and then cream, all at once. Most of the drama is in the last stop. */
  :root[data-theme="miru"], .t-miru {
    color-scheme:light;
    --bg:#504840; --page:var(--bg);
    --stops:#504840 0%,#485848 44%,#fff1cf 88%;
    --veil:#ffffff6b;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(80 20% 9%); --dim:hsl(80 18% 14%); --faint:hsl(80 14% 23%);
    --accent:hsl(95 30% 24%); --accent-2:hsl(35 26% 32%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(80 22% 15% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#ece7d8;
  }

  /* 黒茶鼠 → 露草色 → 鴇羽色. Black tea, then a dayflower blue, then an ibis
     pink — three stops that share nothing, which is the whole idea. The veil
     is set by the tea and costs the other two some strength; there is no
     version of this where all three keep everything. */
  :root[data-theme="kurocha"], .t-kurocha {
    color-scheme:light;
    --bg:#403830; --page:var(--bg);
    --stops:#403830 0%,#38a1db 46%,#f58f84 88%;
    --veil:#ffffff75;
    --surface:#00000012; --raised:#0000001c; --line:#00000030;
    --fg:hsl(25 24% 9%); --dim:hsl(25 20% 15%); --faint:hsl(25 16% 24%);
    --accent:hsl(203 56% 28%); --accent-2:hsl(28 30% 30%); --on-accent:#fff;
    --warn:hsl(350 82% 30%); --warn-bg:#00000012; --ok:hsl(150 62% 20%);
    --shadow:0 1px 3px hsl(25 24% 16% / .2);
    --field:#00000017; --edge:#00000026; --chrome:#eae0dc;
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
    --accent:#bc2d29; --accent-2:hsl(228 45% 38%); --on-accent:#fff;
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
    --accent:hsl(155 55% 22%); --accent-2:hsl(150 26% 58%); --on-accent:#fff;
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
    --accent:hsl(14 55% 30%); --accent-2:hsl(28 40% 64%); --on-accent:#fff;
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
    /* Hex, not the `rgb(236,226,208)` this was lifted from: the commas inside
       a colour function are indistinguishable from the ones between stops for
       anything reading this list, and something does now. */
    --stops:#ece2d0 0%,#fff 70%;
    --gradient-default:circular;
    --surface:hsl(0 0% 96.1%); --raised:hsl(0 0% 93%); --line:hsl(0 0% 89.8%);
    --fg:hsl(0 0% 3.9%); --dim:hsl(0 0% 38%); --faint:hsl(0 0% 46%);
    --accent:hsl(0 0% 9%); --accent-2:hsl(40 40% 68%); --on-accent:hsl(0 0% 98%);
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
    --accent:hsl(220 72% 30%); --accent-2:hsl(216 40% 26%); --on-accent:#fff;
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
  /* The other three sweep the whole page. This one leans in from one corner:
     the ending shape stops at roughly three quarters of the box from low on
     the left, and a radial holds its last stop past that — so the palette
     crowds into that corner and the pale end it finishes on fills everything
     else. Blurred, it reads as light leaking in rather than as a gradient,
     which is the point. Sized rather than cut to transparent: `--stops` ends
     on its own `100%`, and a position after it would collide. */
  :root[data-gradient="air"] {
    --wash:radial-gradient(78% 68% at 4% 72%,var(--stops,var(--no-stops)));
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
  .icon, .proj, .agent, .server, .new button, .theme button,
  .ask button, .sheet button, .chip button, #stale button {
    transition: transform var(--dur-fast) var(--ease),
                background-color var(--dur-fast) var(--ease),
                opacity var(--dur-fast) var(--ease);
  }
  .icon:active, .new button:active, .theme button:active,
  .ask button:active, .sheet button:active, #stale button:active {
    transform:scale(.94);
  }
  /* Full-width rows scale less: the same 6% reads as a lurch across 340px. */
  .proj:active, .agent:active, .server:active { transform:scale(.985); }

  /* ---- header ---------------------------------------------------------- */
  /* Lifted out of the column and laid over it, so the conversation runs
     underneath instead of stopping short — which is the whole point of a
     blurred bar. Nothing passes behind a bar that nothing can reach.

     The blur is only visible because of the 60%: at full opacity there is
     nothing to see through, and `backdrop-filter` would be doing work no one
     could tell was happening. `--bg` is the tint because it is the top of the
     wash, which is the part of the page this sits on. */
  header {
    position:absolute; top:0; left:0; right:0; z-index:4;
    display:flex; align-items:center; gap:11px;
    padding:max(9px,env(safe-area-inset-top)) 10px 9px 14px;
    background:color-mix(in srgb, var(--head-tint,var(--bg)) 60%, transparent);
    backdrop-filter:blur(18px) saturate(150%);
    -webkit-backdrop-filter:blur(18px) saturate(150%);
    /* `--depth` without the ring: the ring would box all four sides, and only
       the underside of a header is an edge. */
    border-bottom:0; box-shadow:var(--depth);
    color:var(--on-head,var(--fg));
  }
  /* The same for the bottom edge, and for the same reason — the composer's
     `backdrop-filter` had nothing behind it to blur while the log ended above
     it. `#ask` and the sheet ride along: they belong to the bottom of the
     screen, not to a position in the transcript. */
  .foot {
    position:absolute; left:0; right:0; bottom:0; z-index:4;
    display:flex; flex-direction:column;
  }
  .who { display:flex; flex-direction:column; min-width:0; flex:1; gap:1px; }
  .who b { font-size:13px; font-weight:500; letter-spacing:-.045em; }
  .who span {
    font-size:9.5px; color:color-mix(in srgb, var(--on-head,var(--fg)) 66%, transparent);
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

  /* 44px, which is the touch target rather than a size chosen by eye, and a
     glyph big enough to be worth the target. The colour comes from the
     header's own composite — see headInk(): these sit on a 60% tint over the
     top of the wash, which is a different surface from the page the rest of
     `--fg` was picked against. */
  .icon {
    flex:none; width:44px; height:44px; border-radius:11px; position:relative;
    display:grid; place-items:center; font-size:19px;
    background:none; border:0; color:var(--on-head,var(--fg));
  }
  .icon:active { background:color-mix(in srgb, var(--on-head,var(--fg)) 10%, transparent); }
  .icon .badge {
    position:absolute; top:2px; right:1px; min-width:16px; height:16px; padding:0 4px;
    background:var(--warn); color:#1a1305; border-radius:999px;
    font-size:8px; font-weight:500; line-height:16px; text-align:center;
  }

  /* ---- conversation ---------------------------------------------------- */
  /* The only child still in the column, so it takes the whole height, and the
     two floating stacks are held off by padding rather than by layout. Both
     heights are measured — the header's grows with the safe area, and the
     foot's with whatever the sheet, a prompt or an attachment adds to it. The
     fallbacks are what they measure to on a phone with neither. */
  #log {
    flex:1; overflow-y:auto; -webkit-overflow-scrolling:touch;
    padding:calc(var(--head,54px) + 12px) 14px calc(var(--foot,88px) + 6px);
  }
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
    background:color-mix(in srgb, var(--accent) var(--bubble-opacity,40%), transparent);
    color:var(--on-bubble,var(--fg)); padding:7px 11px;
    border-radius:12px 12px 3px 12px;
    box-shadow:var(--lift);
  }
  :root[data-bubbles="on"] .row.you .msg code { background:#ffffff2e; }
  /* The agent gets the palette's *other* colour — literally so where there is
     one, as with cinnabar's raven or dusk's silk. It keeps `--fg` rather than
     taking a foreground of its own: this is the body of the conversation, and
     a tint behind it should not also restate what colour it is written in. */
  :root[data-bubbles="on"] .row:not(.you) .msg {
    background:color-mix(in srgb, var(--accent-2,var(--accent)) var(--bubble-opacity,40%), transparent);
    color:var(--on-bubble-2,var(--fg));
    padding:7px 11px; border-radius:12px 12px 12px 3px;
    box-shadow:var(--lift);
  }
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
  /* The ring in place of the border it used to have: this sits inside a
     bubble that is already ringed, and two hard 1px edges a few pixels apart
     is the one thing the layered treatment exists to avoid. */
  .msg pre {
    margin:8px 0 4px; padding:9px 11px; border-radius:8px;
    background:var(--surface); box-shadow:0 1px 2px #00000005, var(--hairline);
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
    border-radius:9px; border:0; background:var(--surface);
    font-size:12px; line-height:1.25; box-shadow:var(--lift);
  }
  .ask button.first { background:var(--accent); color:var(--on-accent); font-weight:500; }
  .ask button:active { transform:scale(.985); }
  .ask .key { opacity:.55; font-weight:500; margin-right:7px; }

  /* ---- composer -------------------------------------------------------- */
  /* One floating card rather than a bar: the field and everything that acts
     on it belong to the same object, and a card can sit over the conversation
     instead of cutting it off with a rule. The padding still resolves the
     safe area, which is what keeps it against the bottom edge. */
  .composer {
    flex:none; padding:6px 10px calc(8px + env(safe-area-inset-bottom));
    background:transparent;
  }
  /* Glassy: a fill that lets the wash through, blurred behind so the card
     reads as a pane over the page rather than a patch of a different colour.
     `backdrop-filter` is the whole effect, so the fill stays translucent.

     It takes the same strength as the bubbles, because it is the same
     question — how much of the page shows through what sits on it — and two
     sliders that disagree would just be a way to get it wrong. At zero the
     fill goes with them and the card is pure blur inside its ring, which is
     what "plain" should mean here too. */
  .dock {
    position:relative; border-radius:22px; padding:4px 6px 6px;
    backdrop-filter:blur(18px) saturate(150%);
    -webkit-backdrop-filter:blur(18px) saturate(150%);
    /* The ring drops out of `--lift` here: the glass edge below is the edge,
       and a hairline under a specular one is the double edge again. */
    box-shadow:var(--float);
    /* Measured against what the card actually composites to at this strength,
       not assumed from the page — see cardInk(). */
    color:var(--on-card,var(--fg));
  }
  /* The fill is a layer of its own so the strength can be an opacity rather
     than a mix. It has to be: `--surface` is an opaque colour on the solid
     themes and an ~8% ink meant to be laid over the wash on the gradient
     ones, so mixing it toward transparent would scale a pane on half the
     palettes and halve an already-faint tint on the other half. Painted over
     an opaque base it is one material everywhere, and the slider then means
     the same thing on every palette.

     The base is `--chrome`, not `--bg`. `--bg` is the *first* stop of a
     gradient theme's wash, and this card sits at the far end of it — on
     porcelain that made a salmon pane at the foot of a page that had long
     since turned pale green. `--chrome` is each theme's bar material, which
     is what this is, so it lands where the page actually is and `--fg` stays
     readable on it right up to a fully solid card.

     Behind the content but inside the card: `backdrop-filter` gives .dock a
     stacking context, so `z-index:-1` reaches the bottom of the card and no
     further, and it is not itself blurred — a backdrop is what lies under an
     element, not what it contains. */
  .dock::before {
    content:""; position:absolute; inset:0; z-index:-1; border-radius:inherit;
    background:linear-gradient(var(--surface),var(--surface)), var(--chrome);
    opacity:var(--bubble-alpha,.4);
  }
  /* The glass edge. Real glass catches light on the rim that faces it and
     lets a little back on the far one, which is why this is a gradient and
     not a border: bright at the top left, gone by the middle, a fainter
     return along the bottom. Drawn as a ring by masking the fill out of its
     own padding box — `xor` leaves exactly the 1px frame.

     White at both ends whatever the palette: a reflection is the colour of
     the light, not of the thing reflecting it, so it stays subtle on a pale
     card and reads clearly on a dark one, which is how glass behaves. */
  .dock::after {
    content:""; position:absolute; inset:0; border-radius:inherit;
    pointer-events:none; padding:1px;
    background:linear-gradient(150deg,
      #ffffffa8 0%, #ffffff38 22%, #ffffff0f 42%,
      transparent 58%, #ffffff1f 88%, #ffffff4d 100%);
    -webkit-mask:linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
    -webkit-mask-composite:xor;
    mask:linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
    mask-composite:exclude;
  }
  /* The tools sit under the field, as a row of their own — where a phone's
     thumb is, and out of the way of the text as it grows. */
  .tools { display:flex; align-items:center; gap:8px; padding:0 2px; }
  .tools .gap { flex:1; }
  .tool {
    position:relative; flex:none; width:36px; height:36px; border:0; padding:0;
    border-radius:50%; background:none; color:var(--on-card,var(--fg));
    display:grid; place-items:center;
    transition:background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease),
               opacity var(--dur-fast) var(--ease), transform var(--dur-fast) var(--ease);
  }
  /* 36px is the circle; the touch target is 44px, and the 8px gaps mean two
     of them meet without ever overlapping. */
  .tool::after { content:""; position:absolute; inset:-4px; border-radius:50%; }
  .tool:active { transform:scale(.94); }
  #mic { background:color-mix(in srgb, var(--on-card,var(--fg)) 10%, transparent); }
  #mic.on { background:color-mix(in srgb, var(--warn) 20%, transparent); color:var(--warn); }
  .tool.send { background:var(--accent); color:var(--on-accent); }
  .tool.send:disabled { opacity:.35; }
  /* Which agent this box talks to, in the place the Claude app puts the
     model — the one choice you make about a message that is not its text. */
  .picker {
    display:flex; align-items:baseline; gap:6px; min-width:0; max-width:62%;
    padding:8px 12px; border-radius:999px; border:0;
    background:color-mix(in srgb, var(--on-card,var(--fg)) 10%, transparent);
    color:var(--on-card,var(--fg)); font-size:11px; font-weight:500; line-height:1;
    transition:background var(--dur-fast) var(--ease), transform var(--dur-fast) var(--ease);
  }
  .picker span {
    color:color-mix(in srgb, var(--on-card,var(--fg)) 62%, transparent);
    font-weight:400; min-width:0;
    overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  }
  .picker:active { transform:scale(.97); }
  /* The mic rides inside the field rather than beside it: dictation is a way
     of filling this box, not a third peer of send and attach, and one fewer
     circle in the row is one fewer thing to read. */
  /* No box of its own: the card is the box. The field is just the top of it,
     so it carries no fill, no edge and no focus ring — focus is already
     obvious from the caret and the keyboard. */
  textarea {
    display:block; width:100%; resize:none; max-height:118px;
    padding:9px 8px 7px; border:0; background:none; color:var(--on-card,var(--fg));
    /* 16px keeps iOS from zooming the page when the field takes focus. */
    font-family:var(--mono); font-size:16px; line-height:1.35; letter-spacing:-.03em;
  }
  textarea:focus { outline:none; }
  textarea::placeholder { color:color-mix(in srgb, var(--on-card,var(--fg)) 42%, transparent); }
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
    border:0; background:var(--surface); color:var(--fg); box-shadow:var(--lift);
  }
  .sheet button.cancel { font-weight:400; color:var(--dim); }

  /* Inside the card, above the field — the attachment belongs to the message
     being written, so it travels with the box rather than floating over it. */
  #attached { display:flex; flex-wrap:wrap; gap:6px; padding:0 2px; }
  #attached:not(:empty) { padding:4px 2px 2px; }
  .chip {
    display:flex; align-items:center; gap:7px; max-width:100%;
    padding:6px 8px 6px 10px; border-radius:8px;
    background:var(--surface); border:0; box-shadow:var(--lift); font-size:10.5px;
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
    border:0; background:none; color:var(--dim); box-shadow:var(--lift);
  }
  .forms button.on { background:var(--raised); color:var(--fg); box-shadow:var(--lift-on); }
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
  /* Was dashed, to say "this makes a new one". The dash cannot survive next
     to the ring without doubling the edge, so the dim label carries it. */
  .new button {
    flex:1; font-size:11px; padding:8px; border-radius:10px;
    border:0; background:none; color:var(--dim); box-shadow:var(--lift);
  }
  .notify {
    flex:none; margin:8px 16px 0; padding:11px 13px; text-align:left;
    border:0; border-radius:11px; background:var(--bg);
    color:var(--fg); font-size:11.5px; line-height:1.3; box-shadow:var(--lift);
  }
  .notify.on { box-shadow:var(--depth), 0 0 0 1px var(--ok); color:var(--dim); }
  /* Five themes will not fit a segmented control, so each is a swatch of the
     colour it actually is — which says more than its name does anyway. */
  .theme { display:flex; flex-wrap:wrap; gap:6px; }
  .theme button {
    display:flex; align-items:center; gap:7px; padding:7px 11px 7px 8px;
    border:0; border-radius:999px; background:none;
    color:var(--dim); font-size:10.5px; font-weight:500; box-shadow:var(--lift);
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
  .theme button.on { background:var(--raised); color:var(--fg); box-shadow:var(--lift-on); }

  #stale {
    flex:none; margin:0 10px 8px; padding:11px 13px; border-radius:14px;
    background:var(--warn-bg); border:1px solid var(--warn);
    font-size:11px; line-height:1.3;
  }
  #stale b { display:block; color:var(--warn); margin-bottom:3px; font-weight:500; }
  #stale button {
    margin-top:8px; padding:7px 11px; border-radius:9px; font-size:11px;
    border:0; background:var(--surface); color:var(--fg); box-shadow:var(--lift);
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

<!-- Everything that lives at the bottom edge, in one stack: it floats over the
     conversation rather than beside it, so the log can scroll underneath. -->
<div class="foot">
<div id="stale" hidden></div>
<div id="ask"></div>
<div class="note" id="note" hidden></div>

<div class="sheet" id="sheet">
  <button onclick="pickFile()">Photo or file</button>
  <button onclick="hideSheet(); queueMessage()">Queue as a TODO</button>
  <button class="cancel" onclick="hideSheet()">Cancel</button>
</div>
<div class="composer">
  <input type="file" id="file" accept="image/*,text/*,.pdf,.log,.json" multiple hidden>
  <div class="dock">
    <div id="attached"></div>
    <textarea id="msg" rows="1" placeholder="Message"></textarea>
    <div class="tools">
      <button class="tool" id="queue" onclick="toggleSheet()" title="attach or queue" aria-label="attach or queue">
        <svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor"
             stroke-width="1.9" stroke-linecap="round" aria-hidden="true">
          <path d="M12 5v14M5 12h14"/>
        </svg>
      </button>
      <button class="picker" id="who" onclick="toggleDrawer()" title="switch agent">
        <b id="pname">—</b><span id="pwhat"></span>
      </button>
      <span class="gap"></span>
      <button class="tool" id="mic" onclick="toggleMic()" title="dictate" aria-label="dictate">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor"
             stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="9" y="2.5" width="6" height="11" rx="3"/>
          <path d="M5.5 11a6.5 6.5 0 0 0 13 0"/>
          <path d="M12 17.5V21"/>
        </svg>
      </button>
      <button class="tool send" id="send" onclick="sendMessage()" disabled aria-label="send">
        <svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor"
             stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 19V5M5.5 11.5 12 5l6.5 6.5"/>
        </svg>
      </button>
    </div>
  </div>
</div>
</div>

<div class="scrim" id="paletteScrim" onclick="togglePalette()"></div>
<section class="palette" id="palette">
  <h2>theme</h2>
  <div class="theme" id="theme"></div>
  <h2>gradient</h2>
  <div class="forms" id="forms"></div>
  <h2>surfaces <span id="bubbleValue"></span></h2>
  <input type="range" id="bubbles" min="0" max="100" step="5"
         oninput="setBubbles(this.value)">
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
  ["cinnabar", "Cinnabar"], ["kite", "Kite"], ["porcelain", "Porcelain"],
  ["dayflower", "Dayflower"], ["lacquer", "Lacquer"], ["chestnut", "Chestnut"],
  ["aster", "Aster"], ["asagi", "Asagi"], ["blossom", "Blossom"],
  ["iron", "Iron"], ["akane", "Akane"], ["persimmon", "Persimmon"],
  ["twilight", "Twilight"], ["verdigris", "Verdigris"], ["miru", "Miru"],
  ["kurocha", "Kurocha"], ["indigo", "Indigo"],
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

  // The fills just changed colour, so what reads on them may have too.
  if (typeof setBubbles === "function") setBubbles(store.get("bubbles", "40"));
}

/* Which shape the stops are drawn in, and how far it is blurred. Both are
   about the page rather than about a theme, so they persist across themes —
   picking a new palette should not silently undo how you like to see it. */
const FORMS = [
  ["linear", "Linear"], ["circular", "Circular"], ["angular", "Angular"], ["air", "Air"],
];

document.getElementById("forms").innerHTML = FORMS.map(([name, label]) =>
  `<button onclick="setForm('${name}')" data-form="${name}">${label}</button>`).join("");

function setForm(name) {
  store.set("gradient", name);
  document.documentElement.setAttribute("data-gradient", name);
  document.querySelectorAll("#forms button").forEach(button =>
    button.classList.toggle("on", button.dataset.form === name));
  // The shape decides which stop lands under the composer, so the ink that
  // reads on it can change without a single colour having changed.
  if (typeof setBubbles === "function") setBubbles(store.get("bubbles", "40"));
}

/* A translucent fill has no fixed foreground. At 40% it is a wash and the
   body colour reads over it; near 100% it is the accent itself and wants the
   accent's own. Where that crossover falls is different in every palette, so
   it is measured rather than guessed at with a threshold.

   Against `--bg` — the top of the page. On a gradient the fill actually sits
   on something between the stops, so this is the darkest reading for a light
   theme and the lightest for a dark one: the safe end either way. */
const swatchProbe = document.createElement("span");
swatchProbe.style.display = "none";
document.body.appendChild(swatchProbe);

function toRgb(value) {
  swatchProbe.style.color = value;
  return (getComputedStyle(swatchProbe).color.match(/[\d.]+/g) || [0, 0, 0]).map(Number);
}

function relativeLuminance([r, g, b]) {
  const channel = v => (v /= 255) <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a, b) {
  const [hi, lo] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/* The colour of the wash where the composer sits.
   Not "the last stop": that is only true of the shapes that run downward.
   Linear sweeps top to bottom, circular is centred on the top edge, and a
   conic from 195° reaches ~96% of its sweep at the bottom of the page — all
   three leave their final stop down here. Air does not: it anchors its glow
   at 4% 72%, so the card sits on the *first* stop, the darkest one on most
   palettes, which is the case where dark body text stopped being readable.

   Holding the ink to every stop instead would be worse than guessing: kite
   runs from dark brown to cream, and no single colour clears a useful bar
   against both ends. A theme with no gradient leaves `--stops` transparent,
   and then the page is simply the page. */
function washUnderCard(styles) { return washAt(styles, true); }

function washAt(styles, atFoot) {
  // Checked before it is read: `toRgb` cannot fail — handed something that is
  // not a colour it quietly returns the probe's inherited one, so a stop this
  // could not parse would come back as the body text and be taken for the
  // page. `CSS.supports` is the only way to tell the two apart.
  const colour = token => CSS.supports("color", token) ? toRgb(token) : [];
  const stops = styles.getPropertyValue("--stops").trim().split(",")
    .map(stop => colour(stop.trim().split(/\s+/)[0]))
    .filter(rgba => rgba.length >= 3 && !(rgba.length === 4 && rgba[3] === 0))
    .map(rgba => rgba.slice(0, 3));
  if (!stops.length) return toRgb(styles.getPropertyValue("--bg")).slice(0, 3);
  const form = document.documentElement.getAttribute("data-gradient") || "linear";
  // Air runs the other way round from the rest: its glow is anchored low and
  // left, so the first stop is at the foot and the last is up by the header.
  const last = form === "air" ? 0 : stops.length - 1;
  const first = form === "air" ? stops.length - 1 : 0;
  const stop = stops[atFoot ? last : first];
  // A theme may lay a veil over the whole wash — lacquer's charcoal end only
  // takes dark text because 56% white is sitting on it. Reading the stop raw
  // would judge the ink against a page nobody sees.
  //
  // The empty check is not a formality: most themes declare no veil, and
  // `toRgb("")` does not fail — it resets the probe and hands back whatever
  // colour it inherits, which is the body text. Passing that through would
  // have told every unveiled theme that its page was the colour of its own
  // ink, and the contrast pass that found this reported a flat 1:1.
  const declared = styles.getPropertyValue("--veil").trim();
  if (!declared || !CSS.supports("color", declared)) return stop;
  const veil = toRgb(declared);
  const over = veil.length === 4 ? veil[3] : 1;
  if (veil.length < 3 || over === 0) return stop;
  return stop.map((c, i) => veil[i] * over + c * (1 - over));
}

/* The ink for the composer's card, shifted to suit what it sits on.
   `--fg` is picked to read on the *page*, and the card is only the page while
   the slider is low: wound up it becomes its own material, and under some
   gradient shapes it lands on the dark end of the wash, where a near-black
   body colour is exactly the wrong choice. So work out what the card comes to
   at this strength, and if `--fg` cannot clear the bar on it, walk `--fg`
   toward white or black — whichever way the surface is not — until it does.
   It comes back untouched when it is already fine, which is most palettes. */
const CARD_CONTRAST = 7;

/* Walk `base` away from `surface` until it clears the bar, or as far as it
   gets. Never across: see cardInk() for what letting it cross cost. */
function inkFor(surface, base) {
  if (contrast(base, surface) >= CARD_CONTRAST) return null;
  const toward = relativeLuminance(base) > relativeLuminance(surface)
    ? [255, 255, 255] : [0, 0, 0];
  let ink = base;
  for (let step = 0.1; step <= 1.0001; step += 0.1) {
    const tried = base.map((c, i) => Math.round(c + (toward[i] - c) * step));
    if (contrast(tried, surface) <= contrast(ink, surface)) break;
    ink = tried;
    if (contrast(ink, surface) >= CARD_CONTRAST) break;
  }
  return "rgb(" + ink.join(",") + ")";
}

/* What the header tints itself with, and the ink that then reads on it.
   Not `--bg`. That is the raw first stop, and on the palettes that open on
   something dark — lacquer's charcoal, iron's 鉄色 — a bar of 60% raw stop is
   far darker than the page under it, which is the same stop with a 40-56%
   veil over it. Dark text on that fell to 2.6:1 on lacquer, a hole this
   header dug for itself by picking the wrong colour to be.

   The visible top of the wash is the right tint, so the bar reads as the page
   thickened rather than as a slab, and `--fg` — chosen against exactly that —
   almost always still fits. `inkFor` covers the rest. */
function headSurface() {
  const styles = getComputedStyle(document.documentElement);
  const tint = washAt(styles, false);
  document.documentElement.style.setProperty("--head-tint", "rgb(" + tint.join(",") + ")");
  return { styles, tint };
}

function headInk() {
  const { styles, tint } = headSurface();
  return inkFor(tint, toRgb(styles.getPropertyValue("--fg")).slice(0, 3)) || "var(--fg)";
}

function cardInk(alpha) {
  const styles = getComputedStyle(document.documentElement);
  const surface = toRgb(styles.getPropertyValue("--surface"));
  const chrome = toRgb(styles.getPropertyValue("--chrome")).slice(0, 3);
  // `--surface` is an ink on some palettes and a colour on others, so lay it
  // over `--chrome` first — the same material the card paints.
  const own = surface.length === 4 ? surface[3] : 1;
  const material = chrome.map((c, i) => surface[i] * own + c * (1 - own));
  const behind = washUnderCard(styles);
  const card = material.map((c, i) => c * alpha + behind[i] * (1 - alpha));

  // Further from the card, never across it. Letting the walk pick whichever
  // direction scored higher is what a first pass did, and it inverted two
  // palettes: indigo's white body colour went black on its blue, which is a
  // theme built around white text, and kite on air went white — the model
  // says that corner is dark brown, the blur says otherwise, and white text
  // on a pale card is a worse answer than the one being fixed. Staying on the
  // side `--fg` already sits makes the worst case "no change", which is what
  // a correction working off an estimate should degrade to.
  const base = toRgb(styles.getPropertyValue("--fg")).slice(0, 3);
  return inkFor(card, base) || "var(--fg)";
}

function readableOn(token, alpha) {
  const styles = getComputedStyle(document.documentElement);
  const page = toRgb(styles.getPropertyValue("--bg"));
  const tint = toRgb(styles.getPropertyValue(token)).map((c, i) => c * alpha + page[i] * (1 - alpha));
  const body = toRgb(styles.getPropertyValue("--fg"));
  const own = toRgb(styles.getPropertyValue("--on-accent"));
  return contrast(own, tint) > contrast(body, tint) ? "var(--on-accent)" : "var(--fg)";
}

/* How solid everything laid over the page is: both speakers' bubbles and the
   composer's card. Zero is not "a bubble you cannot see" — it is no bubble, so
   the padding and the corners go with it. Everything above is a fill at that
   strength. */
function setBubbles(percent) {
  percent = Number(percent) || 0;
  store.set("bubbles", percent);
  const root = document.documentElement;
  root.style.setProperty("--bubble-opacity", percent + "%");
  // The same strength as a number, for the composer card — which scales a
  // whole layer's opacity rather than mixing one colour toward transparent.
  root.style.setProperty("--bubble-alpha", percent / 100);
  root.style.setProperty("--on-bubble", readableOn("--accent", percent / 100));
  root.style.setProperty("--on-bubble-2", readableOn("--accent-2", percent / 100));
  root.style.setProperty("--on-card", cardInk(percent / 100));
  root.style.setProperty("--on-head", headInk());
  root.setAttribute("data-bubbles", percent > 0 ? "on" : "off");
  document.getElementById("bubbles").value = percent;
  document.getElementById("bubbleValue").textContent = percent ? percent + "%" : "plain";
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
  // The composer's pill says who the message is going to. Provider and
  // project only: what the agent is *doing* belongs in the header, not on a
  // control you are about to press.
  document.getElementById("pname").textContent = a.provider;
  document.getElementById("pwhat").textContent = a.project;

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
const bubbleSetting = store.get("bubbles", "40");
setBubbles(bubbleSetting === "on" ? 40 : bubbleSetting === "off" ? 0 : bubbleSetting);
setBlur(store.get("blur", "100"));
setTheme(localStorage.getItem("theme") || "system");
markPush(store.get("push", "") === "on");
/* The header and the foot float over the log, so the log has to be told how
   much of itself they are covering. Measured rather than assumed: the
   header's height moves with the safe area, and the foot's with the sheet
   opening, a prompt arriving, an attachment, or the field growing a line.
   A watcher rather than a one-off for exactly those reasons. */
(function trackEdges() {
  const root = document.documentElement;
  const parts = [["--head", document.querySelector("header")],
                 ["--foot", document.querySelector(".foot")]];
  const measure = () => {
    for (const [name, el] of parts) {
      if (el) root.style.setProperty(name, Math.round(el.getBoundingClientRect().height) + "px");
    }
  };
  measure();
  if (window.ResizeObserver) {
    const watch = new ResizeObserver(measure);
    for (const [, el] of parts) if (el) watch.observe(el);
  }
  addEventListener("resize", measure);
})();
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
