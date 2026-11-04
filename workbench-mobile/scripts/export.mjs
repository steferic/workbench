import { cp, mkdir, rm, readdir, readFile, writeFile } from "node:fs/promises"
import { resolve } from "node:path"
import { createHash } from "node:crypto"

const root = resolve(import.meta.dirname, "..")
const destination = resolve(root, "../assets/mobile")
// Check this before a release to catch source edits that were not bundled.
const hash = createHash("sha256")
async function hashTree(dir) {
  for (const entry of (
    await readdir(resolve(root, dir), { withFileTypes: true })
  ).sort((a, b) => a.name.localeCompare(b.name))) {
    const path = `${dir}/${entry.name}`
    if (entry.isDirectory()) await hashTree(path)
    else {
      hash.update(path)
      hash.update(await readFile(resolve(root, path)))
    }
  }
}
for (const dir of ["app", "components", "hooks", "lib"]) await hashTree(dir)
for (const file of [
  "package.json",
  "package-lock.json",
  "next.config.ts",
  "postcss.config.mjs",
  "components.json",
]) {
  hash.update(file)
  hash.update(await readFile(resolve(root, file)))
}
const fingerprint = hash.digest("hex") + "\n"
if (process.argv.includes("--check")) {
  if (
    (await readFile(resolve(destination, "source.sha256"), "utf8")) !==
    fingerprint
  ) {
    throw Error(
      "The mobile export is stale. Run npm run bundle and commit assets/mobile with the source changes."
    )
  }
  console.log("Mobile export matches its source.")
  process.exit(0)
}
await rm(destination, { recursive: true, force: true })
await mkdir(destination, { recursive: true })
await cp(resolve(root, "out/index.html"), resolve(destination, "index.html"))
await cp(
  resolve(root, "out/_next/static"),
  resolve(destination, "_next/static"),
  { recursive: true }
)
await writeFile(resolve(destination, "source.sha256"), fingerprint)
console.log(
  "Exported mobile app to assets/mobile. Rebuild Workbench to embed it."
)
