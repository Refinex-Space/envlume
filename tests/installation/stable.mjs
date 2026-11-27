// Native developer-layout acceptance. Windows uses a disposable runner's Known Folder.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("../../", import.meta.url));
const windows = process.platform === "win32";
if (windows && process.env.T14_DISPOSABLE_PROFILE !== "1")
  throw Error(
    "Run Windows acceptance in a disposable OS profile with T14_DISPOSABLE_PROFILE=1.",
  );
const tmp = mkdtempSync(join(tmpdir(), "envlume-T14-"));
const profile = join(tmp, "用户 profile");
const workspace = join(tmp, "项目 à");
const other = join(tmp, "unrelated");
for (const p of [profile, workspace, other]) mkdirSync(p);
const env = { ...process.env, ...(windows ? {} : { HOME: profile }) };
const minimal = Object.fromEntries(
  Object.entries(env).filter(([key]) => key.toUpperCase() !== "PATH"),
);
minimal.PATH = "";
const receipt = {};
const snapshot = process.env.T14_TEST_SNAPSHOT;
function run(exe, args, { cwd = other, environment = env, ok = true } = {}) {
  const r = spawnSync(exe, args, {
    cwd,
    env: environment,
    encoding: "utf8",
    timeout: 180000,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (r.error) throw r.error;
  assert.equal(
    r.status === 0,
    ok,
    basename(exe) +
      " " +
      args[0] +
      " returned " +
      r.status +
      ": " +
      r.stderr.slice(-1200),
  );
  return r.stdout.trim();
}
const json = (...args) => JSON.parse(run(...args));
function cargo(args, cwd = repo) {
  const r = spawnSync("cargo", args, {
    cwd,
    env: process.env,
    stdio: ["ignore", "pipe", "inherit"],
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  assert.equal(r.status, 0, "Cargo fixture build failed");
  return r.stdout;
}
const host = /^host: (.+)$/m
  .exec(run("rustc", ["-vV"], { environment: process.env }))[1]
  .trim();
const product = resolve(process.argv[2]);
const sha = (file) =>
  createHash("sha256").update(readFileSync(file)).digest("hex");
const archive = join(repo, "apps/desktop/src-tauri/.cli-stage", host, "debug");
const latest = JSON.parse(readFileSync(join(archive, "release.json")));
const oldVersion = "0.0.0-t14.1";
const targetDir = JSON.parse(
  cargo(["metadata", "--no-deps", "--format-version", "1"]),
).target_directory;
function build(name, kind, cwd, crate) {
  const rows = cargo(
    [
      "build",
      "--locked",
      "--target",
      host,
      "--target-dir",
      targetDir,
      "-p",
      crate,
      kind,
      name,
      "--message-format=json-render-diagnostics",
    ],
    cwd,
  )
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  return rows.find((row) => row.target?.name === name && row.executable)
    .executable;
}
// Build the real CLI sources at a distinct package version. Use a separate bin target
// so verification cannot overwrite the developer's current product binary.
function oldRelease() {
  const src = join(tmp, "previous-src");
  mkdirSync(src);
  copyFileSync(
    join(repo, "rust-toolchain.toml"),
    join(src, "rust-toolchain.toml"),
  );
  cpSync(join(repo, "crates"), join(src, "crates"), { recursive: true });
  const desktop = join(src, "apps/desktop/src-tauri");
  mkdirSync(join(desktop, "src"), { recursive: true });
  copyFileSync(
    join(repo, "apps/desktop/src-tauri/Cargo.toml"),
    join(desktop, "Cargo.toml"),
  );
  writeFileSync(join(desktop, "src/main.rs"), "fn main() {}\n");
  writeFileSync(
    join(src, "Cargo.toml"),
    readFileSync(join(repo, "Cargo.toml"), "utf8").replace(
      'version = "' + latest.version + '"',
      'version = "' + oldVersion + '"',
    ),
  );
  writeFileSync(
    join(src, "Cargo.lock"),
    readFileSync(join(repo, "Cargo.lock"), "utf8")
      .replaceAll("\r\n", "\n")
      .replace(
        /(\[\[package\]\]\nname = "envlume[^"]*"\nversion = ")[^"]+/g,
        "$1" + oldVersion,
      ),
  );
  const manifest = join(src, "crates/cli/Cargo.toml");
  writeFileSync(
    manifest,
    readFileSync(manifest, "utf8") +
      '\n[[bin]]\nname="envlume-t14-previous"\npath="src/main.rs"\n',
  );
  const exe = build("envlume-t14-previous", "--bin", src, "envlume");
  assert.equal(
    run(exe, ["--version"], { environment: minimal }),
    "envlume " + oldVersion,
  );
  const descriptor = join(tmp, "old.json");
  writeFileSync(
    descriptor,
    JSON.stringify({ ...latest, version: oldVersion, sha256: sha(exe) }),
  );
  return [descriptor, exe];
}
async function mcp(config, cwd, callback, refuse = false) {
  const p = spawn(config.command, config.args, {
    cwd,
    env: minimal,
    stdio: "pipe",
  });
  const closed = once(p, "close");
  const reader = createInterface({ input: p.stdout, crlfDelay: Infinity })[
    Symbol.asyncIterator
  ]();
  const timer = setTimeout(() => p.kill(), 45000);
  let id = 0;
  const send = (frame) =>
    p.stdin.write(JSON.stringify({ jsonrpc: "2.0", ...frame }) + "\n");
  async function rpc(method, params) {
    send({ id: ++id, method, params });
    const next = await reader.next();
    assert.ok(!next.done, "unexpected MCP EOF");
    const r = JSON.parse(next.value);
    assert.equal(r.id, id);
    return r;
  }
  try {
    const r = await rpc("initialize", {
      protocolVersion: "2025-11-25",
      clientInfo: { name: "T14", version: "1" },
      capabilities: {},
    });
    if (refuse) {
      assert.equal(r.error.data.errorCode, "PROJECT_NOT_TRUSTED");
    } else {
      assert.equal(r.result.serverInfo.name, "envlume");
      send({ method: "notifications/initialized" });
      const context = async () => {
        const { result } = await rpc("tools/call", {
          name: "project_context",
          arguments: {},
        });
        assert.equal(result.isError, false);
        return result.structuredContent.result.workspace;
      };
      await callback(context);
    }
    p.stdin.end();
    const [code] = await closed;
    assert.equal(code, refuse ? 3 : 0);
  } finally {
    clearTimeout(timer);
    if (p.exitCode === null) p.kill();
  }
}

try {
  const base = join(
    tmp,
    "应用 staged",
    ...(windows ? [] : ["Envlume.app", "Contents", "MacOS"]),
  );
  mkdirSync(base, { recursive: true });
  const desktop = join(base, basename(product));
  copyFileSync(product, desktop);
  const manage = (op, ok = true) =>
    json(desktop, ["--cli", op], { environment: minimal, ok });
  assert.equal(
    manage("locate", false).errorCode,
    "NOT_INSTALLED",
    "fixture requires an unused stable CLI name",
  );
  const seed = build("t14_seed", "--example", repo, "envlume-storage");
  const tty = build("workspace_trust", "--test", repo, "envlume");
  const old = oldRelease();
  const unicode = json(seed, [join(tmp, "本地 CLI à"), ...old]);
  assert.equal(
    run(unicode.path, ["--version"], { environment: minimal }),
    "envlume " + oldVersion,
  );
  const initial = json(seed, ["--local", ...old]);
  const path = initial.path;
  assert.ok(isAbsolute(path));
  assert.equal(manage("locate").cli.release.version, oldVersion);
  run(tty, ["approve", path, "project", "add", workspace, "--json"], {
    cwd: workspace,
  });
  const binding = json(
    path,
    ["project", "inspect", "--path", workspace, "--json"],
    { cwd: workspace, environment: minimal },
  ).binding.bindingId;
  const configFile = join(tmp, "host.json");
  const configuration = JSON.stringify({
    command: path,
    args: ["mcp", "--binding", binding],
  });
  writeFileSync(configFile, configuration);
  const loadConfig = () => JSON.parse(readFileSync(configFile, "utf8"));
  let alias;
  await mcp(loadConfig(), workspace, async (context) => {
    alias = await context();
    if (windows) {
      assert.equal(manage("install", false).errorCode, "BUSY");
      assert.equal(manage("locate").cli.release.version, oldVersion);
      assert.equal(await context(), alias);
    } else {
      assert.equal(manage("install").cli.release.sha256, latest.sha256);
      assert.equal(await context(), alias);
    }
  });
  assert.equal(manage("install").cli.path, path);
  assert.deepEqual(manage("locate").cli, { path, release: latest });
  assert.equal(sha(path), latest.sha256);
  assert.equal(
    run(path, ["--version"], { environment: minimal }),
    "envlume " + latest.version,
  );
  await mcp(loadConfig(), workspace, async (context) =>
    assert.equal(await context(), alias),
  );
  await mcp(loadConfig(), other, undefined, true);
  assert.equal(manage("restore").cli.release.version, oldVersion);
  assert.equal(
    run(path, ["--version"], { environment: minimal }),
    "envlume " + oldVersion,
  );
  await mcp(loadConfig(), workspace, async (context) =>
    assert.equal(await context(), alias),
  );
  assert.equal(manage("install").cli.release.sha256, latest.sha256);
  assert.equal(readFileSync(configFile, "utf8"), configuration);
  Object.assign(receipt, {
    target: host,
    previous: oldVersion,
    current: latest.version,
    maxSchema: latest.maxSchema,
    sha256: latest.sha256,
    emptyPath: true,
    sameConfig: true,
    wrongOrigin: "PROJECT_NOT_TRUSTED",
    liveUpdate: windows ? "BUSY_then_retry" : "old_session_continued",
    restore: "previous_context_verified",
  });
  console.log("T14 native evidence " + JSON.stringify(receipt));
  if (snapshot)
    writeFileSync(snapshot, JSON.stringify(receipt, null, 2) + "\n", {
      flag: "wx",
    });
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
