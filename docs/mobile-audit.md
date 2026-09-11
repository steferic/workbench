# Mobile page audit — 2026-09-08

The original audit below records the behavior before fixes. All 13 grouped findings have now been addressed in the working tree.

The visual structure is coherent and the basic portrait layout fits narrow screens. The priority is correctness: stale approvals, lost drafts, misdirected attachments, and stale output. Several accessibility and theme problems are also reproducible.

## Implementation and verification

- Approvals carry a prompt ticket and are checked against the PTY reader's current screen immediately before writing. Tickets are retired before IO to prevent duplicate answers after ambiguous write failures.
- Commands receive application acknowledgments. IDs deduplicate retries for ten minutes, and a desktop instance ID prevents uncertain requests from being replayed across a restart. Failed/unknown deliveries retain the draft and a recovery entry; uploads remain attached to their originating agent.
- Each device selects its own conversation. History is bounded to 200 visible entries, unchanged nodes survive refreshes, and metadata-only ticks do not invalidate content ETags.
- Uploads use unique paths, reject oversized or incomplete bodies, and run on a bounded set of HTTP workers with socket and total request deadlines. A slow transfer no longer blocks an unrelated read.
- Light tokens, panel focus/labels, touch targets, hidden controls, dictation, landscape composition, viewport handling, manifest launch URLs, and notification registration feedback were updated.
- The desktop Objectives pane was removed as separately requested. Sessions receives its space; saved layouts migrate, navigation/dividers/help were updated, and Desk decisions retain their detail overlay. Stored objectives and manager behavior remain available.

Validation: **487 Rust tests and 10 JavaScript regression tests pass**. The desktop binaries build successfully. Browser checks used the actual page against an isolated fixture at 320×640, 390×844, and 740×390, covering failed-send recovery, terminal updates, Light panels, hidden controls, keyboard focus containment/restoration, and landscape alignment. Physical iPhone keyboard, installation, and push delivery still need device validation.

The low-level `agent.answer` control command now requires the displayed `prompt.id` as its `prompt` parameter. The built-in mobile page sends it automatically. HTTP command clients also send the desktop instance and command identity headers used by the page.

## Evidence and limits

- Loaded the actual embedded HTML and fonts against an isolated local fixture in Chromium/Brave at 390×844 and 320×640. The fixture never controlled real agents. Verified failed sends, stale terminal output, hidden controls, keyboard focus, and explicit Light theme switching.
- Executed the actual upload and dictation handlers in an isolated JavaScript context with delayed network and speech-result callbacks. Both race/loss cases reproduced.
- `cargo test --offline --bin workbench remote:: --quiet`: **47 passed**. Existing tests cover useful Rust behavior, but do not catch the browser interactions below.
- Findings explicitly distinguish runtime reproductions from source-established failure paths. Actual iOS keyboard geometry, home-screen installation, native dictation, and end-to-end push delivery were not device-tested. Browser viewport resizing does not emulate those behaviors.
- Supporting design packages referenced by the audit skill were unavailable; this report uses the technical checklist and direct measurements. The visual assessment is limited to the inspected screens, not every palette and gradient combination.

## Visual assessment and score

No obvious generic-template problem in the inspected screen. Typography and the conversation-first arrangement fit a terminal companion. Glass and gradients are deliberate, configurable choices; their presence alone is not a defect.

| Dimension | Score / 4 | Main finding |
| --- | ---: | --- |
| Accessibility | 1 | Closed panels receive invisible keyboard focus; controls lack useful names |
| Performance | 2 | Conversation storage grows without a bound; changes rebuild the full log |
| Responsive design | 2 | Portrait fits; landscape is deliberately blocked |
| Theming | 2 | Explicit Light inherits dark panel backgrounds under a dark system theme |
| Visual anti-patterns | 3 | Coherent structure; small secondary text and controls need attention |
| **Total** | **10 / 20** | **Acceptable by the rubric; significant work needed** |

This UI score does not incorporate the command-delivery and approval hazards. There are **13 grouped findings: 7 P1, 6 P2, no P0 or P3**. Address findings 1–4 first, followed by theme/accessibility and network resilience.

## Findings

### 1. [P1] An approval tap can answer a different question

**Category:** Command correctness. **Evidence:** Source-established race.

Location: `src/remote/page.rs:1662`, `src/remote/server.rs:271`, `src/app/handler.rs:1552`.

The page submits only the agent and choice key. The desktop checks that the *current* prompt offers that key, without checking which prompt the phone displayed. If question A is answered at the desktop and question B appears before a delayed phone tap arrives, A's “1” is valid for B too. The existing check prevents typing into an ordinary composer, but does not distinguish two consecutive prompts. Escape is accepted unconditionally.

**Fix:** Publish a prompt generation/identity, send it with the selected option, and reject stale responses. Validate again at the point the input is applied, so queued actions cannot outlive their prompt. Disable repeat submission while an answer is pending and surface stale-answer rejection. Suggested workflow: `/harden`.

### 2. [P1] Failed sends lose the editable draft; success feedback is unreliable

**Category:** Delivery and recovery. **Evidence:** Browser reproduction plus source inspection.

Location: `src/remote/page.rs:1476`, `:1578`, `:1588`, `:1655`, `:1844`; `src/remote/server.rs:495`.

With the fixture returning HTTP 503, Send immediately emptied the composer and left the message as a pending bubble. The error disappeared after four seconds; there was no retry/edit action. Attachments are cleared before acknowledgment as well. Queue draft uses the same destructive read. `post()` catches errors and resolves successfully, so notification setup can mark itself enabled even when subscription registration failed.

Separately, the server returns `{"ok":true}` as soon as it enqueues a command. That does not establish that the app accepted or applied it. Later session lookup or action failures cannot reach the requesting device.

**Fix:** Keep a recoverable, per-agent outbox with explicit pending/failed/accepted states; propagate errors from `post()`. Add command IDs and meaningful app acknowledgments, and make retries idempotent. Subscription state should reflect a successful registration. Suggested workflow: `/harden`.

### 3. [P1] An in-flight upload can attach to the wrong agent

**Category:** Data routing. **Evidence:** Executed actual handlers with a delayed upload response.

Location: `src/remote/page.rs:1520`, `:1611`.

Start uploading a photo for agent A, then switch to B before the response arrives. `pick()` clears attachments, but the old callback subsequently appends A's file to the shared attachment array now shown for B. The probe finished with `current = B` and `/uploads/A/photo.jpg` attached. A multi-file selection can also start subsequent uploads using the newly selected agent.

**Fix:** Capture the destination and compose generation before starting the batch. Keep uploads and attachments in per-agent draft state, or cancel/discard stale completions. Revoke preview object URLs when sending or abandoning attachments, not only when explicitly removing one. Suggested workflow: `/harden`.

### 4. [P1] Terminal output can stay stale while polling succeeds

**Category:** Conversation correctness. **Evidence:** Browser reproduction.

Location: `src/remote/page.rs:2473`.

The render signature includes `a.tail.length`, but no terminal text content. Changing the fixture from one line reading “ORIGINAL terminal output” to one reading “UPDATED terminal output” left the old text onscreen through subsequent polls. It appeared only after another action changed the signature. Once terminal output occupies its fixed tail window, this can hide continuing activity for a long time. Equal-length message replacement can also evade the signature.

**Fix:** Use a content revision or actual content comparison, covering resets as well as appended messages. Preserve scroll position while rendering the new revision. Suggested workflow: `/harden`.

### 5. [P1] Explicit Light theme leaves dark drawers with dark text

**Category:** Theming / accessibility. **Evidence:** Browser reproduction and computed colors.

Location: `src/remote/page.rs:101`; panel backgrounds at `:1140`, `:1219`, `:1251`.

With the system in dark mode, select Light. The explicit Light block does not override `--chrome` or `--edge`, although the system-light media query does. The panel background remained `#12151b` while foreground became `#14161b`. Their contrast is approximately **1.01:1**. Unselected theme buttons use `#5f6779`, approximately **3.22:1** on that background. Ordinary text should reach 4.5:1. [W3C contrast guidance](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html).

**Fix:** Define the complete Light token set consistently for explicit and system selection. Check panel, field, and message colors under both OS preferences. Suggested workflow: `/normalize`.

### 6. [P1] Closed panels remain in keyboard navigation and the accessibility tree

**Category:** Accessibility. **Evidence:** Browser reproduction.

Location: `src/remote/page.rs:1140`, `:1219`, `:1251`, `:1368`, `:1418`, `:2144`.

Panels are hidden only with transforms. Pressing Tab from Send focused the closed palette's Auto button at y≈889 in an 844px-high viewport. The accessibility snapshot included every closed theme control and drawer action. Opening a panel does not manage focus, isolate its controls, or restore focus on close. The header buttons exposed glyph names (“◑”, “◆”, “☰”) rather than their purpose; both range sliders were unnamed.

**Fix:** Make closed panels inert, provide dialog names and focus management, support Escape, and label buttons/sliders explicitly. Theme buttons measured 29px high; enlarge their touch area in the same pass. The 36px composer icons already have expanded pseudo-element hit areas, so their visible size alone is not a defect. Suggested workflow: `/harden`, followed by `/adapt`.

### 7. [P1] Landscape is blocked instead of supported

**Category:** Responsive design / accessibility. **Evidence:** Explicit CSS behavior; not touch-device tested.

Location: `src/remote/page.rs:706`; viewport metadata at `:21`.

A coarse-pointer landscape viewport no taller than 480px gets an opaque “portrait, please” cover over the entire app. This prevents use on a phone mounted sideways. A chat/agent interface has no apparent essential orientation requirement. [W3C orientation guidance](https://www.w3.org/WAI/WCAG21/Understanding/orientation.html).

**Fix:** Provide a compact landscape arrangement and remove the blocking cover. Also remove `maximum-scale=1` and verify text enlargement on actual supported phones; enforcement varies by browser, so a universal zoom failure was not claimed here. [W3C text-resizing guidance](https://www.w3.org/WAI/WCAG21/Understanding/resize-text.html). Suggested workflow: `/adapt`.

### 8. [P2] A slow request can stall the rest of the remote UI

**Category:** Network resilience. **Evidence:** Source-established blocking paths.

Location: `src/remote/page.rs:1476`, `:2487`; `src/remote/server.rs:170`, `:432`, `:495`.

Client POSTs have no timeout and hold a global `busy` flag that causes every poll to return early. A request that remains pending can therefore leave all displayed state frozen. Concurrent POSTs also share one Boolean, so the first completion clears it even when another request is still pending.

On the server, each listener handles requests serially, including reading upload bodies. A slow photo transfer can delay state requests and approvals for every device using that listener. Ordinary command bodies are read without a size limit.

**Fix:** Bound request lifetime and body size; allow status polling independently of writes using command acknowledgments. Use bounded concurrent HTTP handling so one slow body cannot occupy the listener. Suggested workflow: `/harden`.

### 9. [P2] Opening a second device can silently stop the first device's conversation updates

**Category:** Synchronization. **Evidence:** Source-established behavior.

Location: `src/app/handler.rs:1573`, `src/remote/mod.rs:529`, `src/remote/page.rs:2527`.

There is one global `remote_focus`. Only that agent's conversation is published. If device B opens a different agent, device A keeps its old conversation and continues receiving statuses, but no new conversation content. The client deliberately reclaims focus only when `data.open` is null, so this does not recover while B owns focus.

**Fix:** Select the conversation per read request, backed by per-agent caches, rather than one global device-independent focus. At minimum, display an explicit ownership/conflict state instead of apparently live stale history. Suggested workflow: `/harden`.

### 10. [P2] Continuous dictation overwrites previous utterances

**Category:** Input correctness. **Evidence:** Executed actual speech-result handler.

Location: `src/remote/page.rs:1695`.

The result handler rebuilds the draft from the pre-dictation text plus results starting at `event.resultIndex`. That index identifies changed results, not the beginning of the complete transcript. In the probe, “existing draft first sentence” became “existing draft second sentence” when result 1 arrived; the first utterance disappeared.

**Fix:** Accumulate finalized results and append the current interim text, or rebuild from the complete recognition result list. Also bind recognition to its originating draft when switching agents. Suggested workflow: `/harden`.

### 11. [P2] A supposedly hidden project-switch button stays visible

**Category:** Layout / interaction. **Evidence:** Browser reproduction at both viewport widths.

Location: `src/remote/page.rs:1082`, `:2468`.

With one project, `#cycleproj.hidden` was true but its computed display was still `grid`. The author `.act { display:grid }` rule overrides the browser's default hidden presentation. The page shows a no-op switcher and consumes composer space.

**Fix:** Explicitly enforce hidden presentation for these controls, for example `.act[hidden] { display:none }`, and check all other elements whose hidden attribute is toggled. Suggested workflow: `/harden`.

### 12. [P2] Upload naming and size handling can corrupt attachments

**Category:** Data integrity. **Evidence:** Source-established boundary cases.

Location: `src/remote/server.rs:427`, `:432`, `:476`.

The stored filename has one-second timestamp precision plus the sanitized original name. Two uploads with the same resulting name in the same second target the same path, and `std::fs::write` replaces the first file. For a body without a declared length, reading through `take(MAX_UPLOAD)` can save the first 25MB and return success without detecting that additional bytes existed.

**Fix:** Use collision-resistant names and exclusive creation. Read at most limit+1 and reject oversized bodies instead of saving a prefix. Suggested workflow: `/harden`.

### 13. [P2] Long sessions accumulate rendering and polling work

**Category:** Performance. **Evidence:** Source inspection; no device performance benchmark.

Location: `src/remote/page.rs:1506`, `:2331`, `:2380`, `:2483`; `src/remote/mod.rs:582`; `src/remote/server.rs:327`.

The browser continually concatenates messages with no retention bound, then rebuilds the complete log when its signature changes. The project tree and prompt markup are rebuilt on every successful state render. Meanwhile `snapshot.at` changes every second and is included in the ETag, undermining 304 responses even when the useful state is unchanged. Long-lived sessions therefore grow memory/DOM work, while idle sessions still receive changing snapshots.

**Fix:** Bound or virtualize visible history, update message nodes incrementally, preserve interactive nodes when unchanged, and base cache validation on meaningful state revisions. Suggested workflow: `/optimize`.

## What is working

- The inspected portrait layouts had no horizontal document overflow at 320px or 390px. The composer fit, and the theme sheet scrolled within its height cap.
- Header/footer overlap is measured with `ResizeObserver`; the code does not assume a fixed composer height as drafts, prompts, and attachments grow.
- Fonts ship locally. The page does not depend on a public font CDN.
- Conversation transfer uses incremental offsets and epochs, with explicit reset handling and a server-side tail window.
- Polls are protected against overlapping reads and normally have a timeout. Extending that discipline to writes is a focused improvement.
- Terminal/journal text is escaped before being rendered. Upload names are rebuilt instead of directly trusting supplied paths.

## Follow-up device checks

Verify keyboard open/close, rotation, safe areas, installed-home-screen relaunch, token persistence, and push delivery on a real iPhone after fixes. There is a served manifest but no manifest link in the HTML, and its `start_url` is `./` while the root page requires a token. Installation behavior needs an explicit end-to-end check before treating the manifest as working; this review does not assert that existing Safari home-screen installations all fail.

## Recommended order

1. `/harden` — prompt identity, recoverable sends, upload ownership, content revisions, and focus accessibility.
2. `/normalize` — complete the Light tokens and check contrast on both OS themes.
3. `/adapt` — support landscape, accessible zoom, and larger secondary touch targets.
4. `/harden` — network deadlines/concurrency, per-device conversation selection, dictation, and upload integrity.
5. `/optimize` — bound history and avoid unnecessary DOM rebuilding and snapshot transfers.
6. `/polish` — finish spacing, control feedback, and error wording after behavior is reliable.

These passes can be done individually or together. Re-run `/audit` and the browser/device scenarios after fixes.
