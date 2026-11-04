# Workbench mobile

The phone UI is a React 19 / Next.js static export, served by Workbench's existing Rust HTTP server. The sidebar contains projects only. Agents are selected within a project; project actions, jobs, decisions, appearance, and notifications have separate controls.

## Other computers

The phone uses the `[[machines]]` connections already configured on its Workbench Mac. Remote projects appear in the same project list as `Project @ computer`; select one, then use the agent dropdown as usual. Settings → Computers shows each connection. No remote token or separate phone bookmark is needed.

Messages, queued messages, prompt answers, project jobs, new agents, and decisions go through the current Workbench to the owning computer. Attachments (up to 25 MB) are stored on that computer, and explicitly presented image/video previews stream through the gateway, including video range requests. Both Workbench instances must be running and reachable.

When a computer goes offline, its last known projects stay visible with an offline notice. Drafts remain editable and saved per agent; reconnecting never sends them automatically. Pending commands retain their original remote process identity, so a retry cannot silently execute after that computer's Workbench restarts. Review and resend a rejected message when ready.

## Develop

```sh
npm ci
npm run fixture # isolated synthetic backend on 127.0.0.1:4174
# In another terminal:
WORKBENCH_BACKEND=http://127.0.0.1:4174 npm run dev
```

Open `http://localhost:3000/?t=test`. The fixture never controls real agents. To use a live Workbench backend, set `WORKBENCH_BACKEND` to its reachable origin and open the dev app with that backend's phone token. Keep that token private.

## Build and embed

```sh
npm run bundle
npm run check:bundle
cd ..
cargo build --release
```

Commit `assets/mobile` alongside frontend changes. Rust embeds the export, including its JS, CSS, and Inter font; users need no Node process or external asset host. Cargo builds use the committed export without invoking npm. Restart Workbench with the rebuilt binary and reload the phone page to use a changed export. Existing phone links, saved per-agent drafts, pending deliveries, and push subscriptions remain compatible.

Only content-hashed `/_next/static` assets bypass token authentication. The page, API, uploads, media, manifest, and service worker retain the existing authentication rules. Entry HTML is not cached. The source fingerprint check detects an outdated frontend export before release.

## Design system

Installed from Fluid Functionalism's `pill-compact-tabler` Next.js starter, including every component and shared system. `components/providers.tsx` sets Base UI components, pill shape, compact sizing, Tabler icons, system theme, and OS reduced motion. `app/layout.tsx` loads Inter with the `opsz` axis and sets `--fluid-accent: #6B97FF`. Phone wrappers enlarge touch targets to 44px and editable text to 16px without changing the registry's animation system. D switches light/dark outside text fields.

Cuelume supplies quiet action, selection, navigation, and confirmed-result cues. Settings includes a persisted sound toggle. Audio starts only after a user gesture; nothing plays per streamed token or tool call.

Add components with `npx shadcn@latest add @fluid/base/<name>`. Check local diffs before overwriting. One compatibility fix is carried in `components/ui/color-picker.tsx`: the generated starter passed Radix's unsupported `disableHoverableContent` prop to the Base UI tooltip provider; that prop was removed. Other registry components retain their upstream behavior.

## Validate

```sh
npm test                # transport, delivery, drafts, cursors, approvals
npm run typecheck
npm run lint
npm run test:browser    # real Chromium against the static export + fixture
```

Browser tests use installed Chrome/Brave on macOS, `CHROMIUM_PATH` when set, or Playwright's Chromium (`npx playwright install chromium`). Screenshots land in the gitignored `.taste-skill/screens` directory at the repository root. `node --test tests/mobile-page.test.cjs` from the root remains a compatibility entry point for the transport regression suite.

The browser suite covers project-only navigation, empty projects, per-agent drafts, failed-send recovery, prompt identity, explicit themes, mute persistence, modal focus, inline media/back navigation, attachment-only delivery, dictation ownership, remote actions/reconnection, and portrait/landscape layout. Rust integration tests exercise two isolated servers through the gateway, including authentication, upload ownership, preview ranges, command identity, and remote restart rejection. Native iPhone keyboard geometry, dictation permissions, and home-screen push delivery still require a physical device.
