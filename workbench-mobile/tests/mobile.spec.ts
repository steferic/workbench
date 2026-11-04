import { test, expect } from "@playwright/test"
import { mkdir } from "node:fs/promises"

test("dictation accumulates results and cannot write into another project", async ({
  page,
}) => {
  await page.addInitScript(() => {
    class MockRecognition {
      onresult: ((event: unknown) => void) | null = null
      onend: (() => void) | null = null
      start() {
        Object.assign(window, { mockSpeech: this })
      }
      stop() {
        this.onend?.()
      }
    }
    Object.assign(window, { SpeechRecognition: MockRecognition })
  })
  await page.goto("/?t=test")
  await page.getByRole("button", { name: "Dictate message" }).click()
  await page.evaluate(() => {
    const mock = (
      window as unknown as {
        mockSpeech: { onresult: (event: unknown) => void }
      }
    ).mockSpeech
    Object.assign(window, { lateSpeechResult: mock.onresult })
    mock.onresult({
      results: [
        [{ transcript: "First sentence. " }],
        [{ transcript: "Second sentence." }],
      ],
    })
  })
  await expect(page.getByRole("textbox")).toHaveValue(
    "First sentence. Second sentence."
  )
  await page.getByRole("button", { name: "Projects", exact: true }).click()
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "Canvas", exact: true })
    .click()
  await page.evaluate(() => {
    ;(
      window as unknown as { lateSpeechResult: (event: unknown) => void }
    ).lateSpeechResult({ results: [[{ transcript: "Late result" }]] })
  })
  await expect(page.getByRole("textbox")).toHaveValue("")
})

test("an attachment can be sent without text to its owning agent", async ({
  page,
}) => {
  const commands: { url: string; body: string }[] = []
  await page.route("**/api/upload?*", async (route) => {
    expect(new URL(route.request().url()).searchParams.get("agent")).toBe("a1")
    await route.fulfill({ json: { path: "/uploads/a1/context.txt" } })
  })
  await page.route("**/api/reply?*", async (route) => {
    commands.push({
      url: route.request().url(),
      body: route.request().postData() || "",
    })
    await route.fulfill({ json: { ok: true } })
  })
  await page.goto("/?t=test")
  await expect(page.getByRole("textbox")).toBeVisible()
  await page.locator('input[type="file"]').setInputFiles({
    name: "context.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("Attached context"),
  })
  await expect(page.getByLabel("Attachments", { exact: true })).toContainText(
    "context.txt"
  )
  await page
    .getByRole("button", { name: "Send attachments", exact: true })
    .click()
  await expect(
    page.getByLabel("Attachments", { exact: true })
  ).not.toBeVisible()
  expect(commands).toHaveLength(1)
  expect(JSON.parse(commands[0].body).agent).toBe("a1")
  expect(commands[0].body).toContain("/uploads/a1/context.txt")
})

test.beforeEach(async ({ page }) => {
  // Each browser context has its own storage, synthetic state, and commands.
  await page.addInitScript(() => {
    localStorage.setItem("agent", "a1")
    localStorage.setItem("project", "workbench")
  })
})

test("project navigation contains only projects, preserves drafts, and handles empty projects", async ({
  page,
}) => {
  const errors: string[] = []
  page.on("pageerror", (e) => errors.push(e.message))
  await page.goto("/?t=test")
  await expect(
    page.getByRole("textbox", { name: "Message GPT-6" })
  ).toBeVisible()
  await page
    .getByRole("textbox", { name: "Message GPT-6" })
    .fill("Keep my Workbench draft")
  await page.waitForResponse("**/api/state?*")
  await expect(page.getByLabel("Conversation", { exact: true })).toHaveCount(1)
  await expect(page.locator(".chat-agent")).toHaveCount(1)
  await page.getByRole("button", { name: "Projects", exact: true }).click()
  const nav = page.getByRole("navigation", { name: "Projects" })
  await expect(nav.getByRole("button")).toHaveCount(3)
  await expect(nav).not.toContainText("localhost")
  await expect(nav).not.toContainText("Claude")
  await nav.getByRole("button", { name: "Canvas", exact: true }).click()
  await expect(
    page.getByRole("heading", { name: "Canvas", exact: true })
  ).toBeVisible()
  await expect(page.getByRole("textbox")).toHaveValue("")
  await page.getByRole("button", { name: "Projects", exact: true }).click()
  await nav.getByRole("button", { name: "Workbench", exact: true }).click()
  await expect(page.getByRole("textbox")).toHaveValue("Keep my Workbench draft")
  await page.getByRole("button", { name: "Projects", exact: true }).click()
  await nav.getByRole("button", { name: "Field notes", exact: true }).click()
  await expect(
    page.getByRole("heading", { name: "Start in Field notes" })
  ).toBeVisible()
  await expect(page.getByRole("button", { name: "Start Codex" })).toBeVisible()
  await page.waitForTimeout(1200)
  await expect(
    page.getByRole("heading", { name: "Start in Field notes" })
  ).toBeVisible()
  expect(errors).toEqual([])
})

test("failed messages retain text, retry with the same ID, and acknowledge only once", async ({
  page,
}) => {
  const ids: string[] = []
  let fail = true
  await page.route("**/api/reply?*", async (route) => {
    ids.push(route.request().headers()["x-command-id"])
    await route.fulfill({
      status: fail ? 503 : 200,
      body: fail ? "Fixture outage" : '{"ok":true}',
    })
  })
  await page.goto("/?t=test")
  const box = page.getByRole("textbox", { name: "Message GPT-6" })
  await box.fill("A recoverable message")
  await page.getByRole("button", { name: "Send message", exact: true }).click()
  await expect(page.getByRole("button", { name: "Retry safely" })).toBeVisible()
  await expect(box).toHaveValue("A recoverable message")
  fail = false
  await page.getByRole("button", { name: "Retry safely" }).click()
  await expect(box).toHaveValue("")
  expect(ids).toHaveLength(2)
  expect(ids[0]).toBe(ids[1])
})

test("approval actions carry the current prompt ticket", async ({ page }) => {
  const requests: Record<string, string>[] = []
  await page.route("**/api/answer?*", async (route) => {
    requests.push(route.request().headers())
    await route.fulfill({ body: '{"ok":true}' })
  })
  await page.goto("/?t=test")
  await page.getByRole("combobox", { name: "Agent in this project" }).click()
  await page.getByRole("option", { name: "Claude", exact: true }).click()
  await page
    .getByRole("button", { name: "Yes, run the checks", exact: true })
    .click()
  expect(requests).toHaveLength(1)
  expect(requests[0]["x-prompt-id"]).toBe("prompt-1")
})

test("settings follow explicit light theme, mute persists, and dialog focus stays inside", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" })
  await page.goto("/?t=test")
  const settings = page.getByRole("button", { name: "Settings", exact: true })
  await settings.click()
  await page.getByRole("combobox", { name: "Appearance" }).click()
  await page.getByRole("option", { name: "Light", exact: true }).click()
  await expect(page.locator("html")).toHaveClass(/light/)
  const sounds = page.getByRole("switch", { name: "Interaction sounds" })
  await sounds.click()
  await expect(sounds).not.toBeChecked()
  const dialog = page.getByRole("dialog", { name: "Settings" })
  await expect(dialog).toBeVisible()
  for (let i = 0; i < 8; i++) {
    await page.keyboard.press("Tab")
    await expect
      .poll(() =>
        page.evaluate(
          () => !!document.activeElement?.closest('[role="dialog"]')
        )
      )
      .toBe(true)
  }
  await page.keyboard.press("Escape")
  await expect(dialog).not.toBeVisible()
  await expect(settings).toBeFocused()
  await page.reload()
  await settings.click()
  await expect(
    page.getByRole("switch", { name: "Interaction sounds" })
  ).not.toBeChecked()
})

test("media is scoped, escaped, placed chronologically, and closes with browser back", async ({
  page,
  request,
}) => {
  const data = await (await request.get("/api/state")).json()
  data.media = [
    {
      id: "image-1",
      agent: "a1",
      name: "screen <img onerror=oops>.png",
      kind: "image",
      at: "2026-11-04T15:30:20Z",
    },
    { id: "private", agent: "b1", name: "Private image", kind: "image" },
  ]
  await page.route("**/api/state?*", (route) => route.fulfill({ json: data }))
  await page.route("**/media/image-1/file?*", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="400" height="240"><rect width="400" height="240" fill="#6b97ff"/></svg>',
    })
  )
  await page.goto("/?t=test")
  await expect(page.getByText("Private image")).toHaveCount(0)
  const media = page.getByRole("button", {
    name: "View screen <img onerror=oops>.png",
  })
  await expect(media).toBeVisible()
  const text = await page.locator(".transcript").innerText()
  expect(text.indexOf("screen <img")).toBeLessThan(
    text.indexOf("The project switcher is ready")
  )
  await media.click()
  await expect(page.getByRole("dialog")).toBeVisible()
  await page.goBack()
  await expect(page.getByRole("dialog")).not.toBeVisible()
})

for (const [width, height] of [
  [320, 640],
  [390, 844],
  [740, 390],
  [1440, 900],
]) {
  test(`layout fits ${width}×${height} with reachable controls`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height })
    await page.emulateMedia({ colorScheme: "light" })
    await page.goto("/?t=test")
    await expect(
      page.getByRole("textbox", { name: "Message GPT-6" })
    ).toBeVisible()
    const metrics = await page.evaluate(() => ({
      overflow: document.documentElement.scrollWidth > innerWidth,
      send: document
        .querySelector('[aria-label="Send message"]')!
        .getBoundingClientRect()
        .toJSON(),
      font: getComputedStyle(document.querySelector("textarea")!).fontSize,
    }))
    expect(metrics.overflow).toBe(false)
    expect(metrics.send.bottom).toBeLessThanOrEqual(height)
    expect(metrics.send.height).toBeGreaterThanOrEqual(44)
    expect(parseFloat(metrics.font)).toBeGreaterThanOrEqual(16)
    await mkdir("../.taste-skill/screens", { recursive: true })
    await page.screenshot({
      path: `../.taste-skill/screens/conversation-${width}.png`,
    })
    await page.getByRole("button", { name: "Projects", exact: true }).click()
    if (width < 768) {
      await expect(
        page.getByRole("navigation", { name: "Projects" })
      ).toBeVisible()
      await expect
        .poll(async () =>
          Math.abs(
            (await page
              .locator('[data-mobile="true"][data-sidebar="sidebar"]')
              .boundingBox())!.x
          )
        )
        .toBeLessThan(0.1)
      await page.screenshot({
        path: `../.taste-skill/screens/projects-${width}.png`,
      })
    }
  })
}

test("remote projects support agent switching, messages, files, decisions, and reconnecting", async ({
  page,
  request,
}) => {
  const data = await (await request.get("/api/state")).json()
  data.machines = [{ name: "mbp6", status: "online", instance: "remote-1" }]
  data.projects.push({
    ...data.projects[0],
    id: "mbp6~workbench",
    name: "Workbench @ mbp6",
  })
  const remote = data.agents
    .slice(0, 2)
    .map((agent: Record<string, unknown>) => ({
      ...agent,
      id: `mbp6~${agent.id}`,
      project_id: "mbp6~workbench",
      project: "Workbench @ mbp6",
      messages: [
        {
          role: "agent",
          text: "Conversation on the other computer.",
          at: "2026-11-04T15:31:00Z",
        },
      ],
      prompt: agent.prompt
        ? { ...(agent.prompt as object), id: "remote-prompt" }
        : undefined,
    }))
  data.agents.push(...remote)
  data.desk = [
    {
      id: "mbp6~proposal",
      kind: "pending",
      project: "Workbench @ mbp6",
      title: "Publish the remote update",
    },
  ]
  data.media = [
    {
      id: "mbp6~preview",
      agent: "mbp6~a1",
      name: "Remote preview",
      kind: "image",
      at: "2026-11-04T15:32:00Z",
    },
  ]
  await page.route("**/api/state?*", (route) => route.fulfill({ json: data }))
  await page.route("**/media/mbp6~preview/file?*", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="80"><rect width="160" height="80" fill="#6b97ff"/></svg>',
    })
  )
  const commands: {
    path: string
    body: { agent: string; text: string }
    headers: Record<string, string>
  }[] = []
  await page.route(
    /\/api\/(reply|answer|proposal|new-agent)\?/,
    async (route) => {
      commands.push({
        path: new URL(route.request().url()).pathname,
        body: route.request().postDataJSON(),
        headers: route.request().headers(),
      })
      await route.fulfill({ json: { ok: true } })
    }
  )
  await page.route("**/api/upload?*", async (route) => {
    expect(new URL(route.request().url()).searchParams.get("agent")).toBe(
      "mbp6~a1"
    )
    await route.fulfill({ json: { path: "/remote/uploads/context.txt" } })
  })
  await page.goto("/?t=test")
  await page.getByRole("textbox").fill("My local draft")
  await page.getByRole("button", { name: "Projects", exact: true }).click()
  const nav = page.getByRole("navigation", { name: "Projects" })
  await expect(nav.getByRole("button")).toHaveCount(4)
  await page
    .getByRole("button", { name: "Workbench @ mbp6", exact: true })
    .click()
  await expect(
    page.getByText("Conversation on the other computer.")
  ).toBeVisible()
  await expect(page.getByRole("textbox")).toHaveValue("")
  await page.getByRole("combobox", { name: "Agent in this project" }).click()
  await page.getByRole("option", { name: "GPT-6", exact: true }).click()
  const media = page.getByRole("button", { name: "View Remote preview" })
  await expect(media).toBeVisible()
  await media.click()
  await expect(page.getByRole("dialog")).toBeVisible()
  await page.keyboard.press("Escape")
  await page.getByRole("textbox").fill("Work on the remote file")
  await page.locator('input[type="file"]').setInputFiles({
    name: "context.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("context"),
  })
  await expect(page.getByLabel("Attachments", { exact: true })).toContainText(
    "context.txt"
  )
  await page.getByRole("button", { name: "Send message", exact: true }).click()
  await expect(page.getByRole("textbox")).toHaveValue("")
  expect(commands[0].body).toEqual({
    agent: "mbp6~a1",
    text: "Work on the remote file\n/remote/uploads/context.txt",
  })
  expect(commands[0].headers["x-peer-instance"]).toBe("remote-1")
  expect(commands[0].headers["x-workbench-instance"]).toBe("fixture-1")
  await page.getByRole("combobox", { name: "Agent in this project" }).click()
  await page.getByRole("option", { name: "Claude", exact: true }).click()
  await page.getByRole("button", { name: "Yes, run the checks" }).click()
  expect(commands.at(-1)?.headers["x-prompt-id"]).toBe("remote-prompt")
  expect(commands.at(-1)?.body.agent).toBe("mbp6~a2")
  await page.getByRole("button", { name: "Decisions, 1 waiting" }).click()
  await page.getByRole("button", { name: "Approve", exact: true }).click()
  expect(commands.at(-1)?.body.agent).toBe("mbp6~proposal")
  expect(commands.at(-1)?.headers["x-peer-instance"]).toBe("remote-1")
  await page.keyboard.press("Escape")
  await page.getByRole("button", { name: "Project actions" }).click()
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Codex", exact: true })
    .click()
  expect(commands.at(-1)?.body.agent).toBe("mbp6~workbench")
  await page.keyboard.press("Escape")
  data.machines[0].status = "offline"
  await expect(page.getByText("mbp6 is offline", { exact: true })).toBeVisible()
  await page.getByRole("textbox").fill("Keep my remote draft")
  await expect(
    page.getByRole("button", { name: "Send when connected" })
  ).toBeDisabled()
  await expect(
    page.getByRole("button", { name: "Attach files", exact: true })
  ).toBeDisabled()
  await page.waitForTimeout(650) // Let the dismissed dialog's exit spring settle for the screenshot.
  await page.screenshot({
    path: "../.taste-skill/screens/remote-offline-390.png",
  })
  const count = commands.length
  data.machines[0].status = "online"
  await expect(
    page.getByText("mbp6 is offline", { exact: true })
  ).not.toBeVisible()
  await expect(page.getByRole("textbox")).toHaveValue("Keep my remote draft")
  expect(commands).toHaveLength(count)
  await page.getByRole("button", { name: "Settings", exact: true }).click()
  const computers = page.getByRole("region", { name: "Computers" })
  await expect(computers).toContainText("mbp6")
  await expect(computers).toContainText("Connected")
  await page.waitForTimeout(650) // Capture the settled Fluid dialog, not its entrance frame.
  await page.screenshot({ path: "../.taste-skill/screens/computers-390.png" })
  await page.keyboard.press("Escape")
  await page.getByRole("button", { name: "Projects", exact: true }).click()
  await nav.getByRole("button", { name: "Workbench", exact: true }).click()
  await expect(page.getByRole("textbox")).toHaveValue("My local draft")
})
