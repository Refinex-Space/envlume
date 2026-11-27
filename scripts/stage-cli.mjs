// Tauri runs this hook before Cargo. Build the CLI from the same locked workspace.
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  renameSync,
  writeFileSync,
  mkdtempSync,
  rmSync,
} from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const host = /^host: (.+)$/m
  .exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))[1]
  .trim();
const target =
  process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET || host;
if (
  ![
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
  ].includes(target)
)
  throw Error("Unsupported CLI target: " + target);
const debug =
  process.argv.includes("--debug") || process.env.TAURI_ENV_DEBUG === "true";
const args = [
  "build",
  "--locked",
  "-p",
  "envlume",
  "--bin",
  "envlume",
  "--target",
  target,
  "--message-format=json-render-diagnostics",
];
if (!debug) args.push("--release");
const cargo = spawnSync("cargo", args, {
  cwd: root,
  stdio: ["ignore", "pipe", "inherit"],
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
});
if (cargo.status !== 0) process.exit(cargo.status ?? 1);
const messages = cargo.stdout
  .trim()
  .split("\n")
  .map((line) => JSON.parse(line));
const artifact = messages.find(
  (m) =>
    m.reason === "compiler-artifact" &&
    m.target.name === "envlume" &&
    m.executable,
);
const metadata = messages.find(
  (m) =>
    m.reason === "build-script-executed" &&
    m.package_id === artifact.package_id,
);
const release = JSON.parse(
  readFileSync(join(metadata.out_dir, "cli-build.json"), "utf8"),
);
if (release.target !== target)
  throw Error("CLI build reported a different target");
// Probe the native product, without opening a catalogue. Cross builds use the Cargo build identity.
if (
  target === host &&
  execFileSync(artifact.executable, ["--version"], {
    encoding: "utf8",
  }).trim() !==
    "envlume " + release.version
)
  throw Error("CLI version probe disagrees with Cargo");
const bytes = readFileSync(artifact.executable);
release.sha256 = createHash("sha256").update(bytes).digest("hex");
const stage = join(
  root,
  "apps",
  "desktop",
  "src-tauri",
  ".cli-stage",
  target,
  debug ? "debug" : "release",
);
mkdirSync(stage, { recursive: true });
const tmp = mkdtempSync(join(stage, ".copy-"));
try {
  for (const [name, content] of [
    ["image", bytes],
    ["release.json", JSON.stringify(release)],
  ]) {
    writeFileSync(join(tmp, name), content);
    renameSync(join(tmp, name), join(stage, name));
  }
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
console.log(`Staged envlume ${release.version} (${target})`);
