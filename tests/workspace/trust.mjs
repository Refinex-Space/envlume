import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  mkdirSync,
  writeFileSync,
  renameSync,
  symlinkSync,
  unlinkSync,
  rmSync,
  existsSync,
} from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";

const [exe, helper] = process.argv.slice(2);
const root = process.cwd(),
  windows = process.platform === "win32";
function run(program, args, cwd = root, code = 0, input) {
  const r = spawnSync(program, args, {
    cwd,
    encoding: "utf8",
    timeout: 45000,
    input,
    stdio: ["pipe", "pipe", "pipe"],
  });
  assert.equal(
    r.status,
    code,
    JSON.stringify({
      program,
      args,
      status: r.status,
      out: r.stdout,
      err: r.stderr,
      error: r.error?.message,
    }),
  );
  return r.stdout;
}
if (windows) {
  const fs = run("powershell.exe", [
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    "[IO.DriveInfo]::new([IO.Path]::GetPathRoot([Environment]::CurrentDirectory)).DriveFormat",
  ]).trim();
  console.log("Native test volume:", fs);
} else if (process.platform === "darwin") {
  const device = run("df", ["-P", root]).trim().split("\n")[1].split(/\s+/)[0];
  const info = run("diskutil", ["info", device]);
  console.log(
    "Native test volume:",
    /File System Personality:\s*(.+)/.exec(info)[1].trim(),
  );
}
const cli = (...args) => JSON.parse(run(exe, [...args, "--json"]));
const confirm = (...args) => run(helper, ["approve", exe, ...args, "--json"]);
function add(path, project) {
  confirm("project", "add", path, ...(project ? ["--project", project] : []));
  const result = cli("project", "inspect", "--path", path);
  assert.equal(result.binding.state, "TRUSTED");
  assert.equal(result.binding.confirmationSource, "LOCAL_CLI_TTY");
  return result;
}
function link(from, to) {
  symlinkSync(to, from, windows ? "junction" : "dir");
}
function unlink(path) {
  if (windows) rmSync(path);
  else unlinkSync(path);
}
const git = (cwd, ...args) => run("git", args, cwd);
const primary = join(root, "first", "Orders"),
  clone = join(root, "second", "Orders"),
  tree = join(root, "branch-checkout");
mkdirSync(primary, { recursive: true });
mkdirSync(join(root, "second"));
git(primary, "init", "-q");
writeFileSync(join(primary, "pom.xml"), "<project/>");
git(primary, "add", "pom.xml");
git(
  primary,
  "-c",
  "user.name=T05",
  "-c",
  "user.email=t05@example.invalid",
  "commit",
  "-qm",
  "fixture",
);
git(primary, "worktree", "add", "-q", "--detach", tree);
git(root, "clone", "-q", "--local", primary, clone);
for (const p of [primary, clone])
  git(p, "config", "remote.origin.url", "https://example.invalid/shared.git");

const unconfirmed = JSON.parse(
  run(exe, ["project", "add", primary, "--json"], root, 3, "trust\n"),
);
assert.equal(unconfirmed.errorCode, "POLICY_DENIED");
const first = add(primary),
  a = first.binding;
assert.ok(first.workspace.hints.includes("maven"));
assert.equal(first.workspace.gitKind, "repository");
const other = add(clone).binding;
assert.notEqual(other.projectId, a.projectId);
assert.notEqual(other.bindingId, a.bindingId);
const worktree = add(tree, a.projectId);
assert.equal(worktree.workspace.gitKind, "worktree");
assert.equal(worktree.binding.projectId, a.projectId);
assert.notEqual(worktree.binding.bindingId, a.bindingId);
assert.equal(add(primary).binding.bindingId, a.bindingId);
console.log(
  "CLI registration:",
  JSON.stringify({
    state: a.state,
    source: a.confirmationSource,
    git: first.workspace.gitKind,
    hints: first.workspace.hints,
    sameRemoteIndependent: true,
    linkedProjectShared: true,
  }),
);

assert.ok(cli("project", "list").projects.some((p) => p.id === a.projectId));
const init = {
  protocolVersion: "2025-11-25",
  clientInfo: { name: "t05-test", version: "1" },
  capabilities: {},
};
async function bounded(p, label) {
  let timer;
  try {
    return await Promise.race([
      p,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("Timeout: " + label)), 20000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
async function session(cwd, id, exercise, exit = 0) {
  const child = spawn(exe, ["mcp", "--binding", id], { cwd, stdio: "pipe" });
  const closed = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  });
  let err = "",
    out = "",
    count = 0;
  child.stderr.setEncoding("utf8").on("data", (s) => (err += s));
  child.stdout.setEncoding("utf8").on("data", (s) => (out += s));
  const lines = createInterface({ input: child.stdout })[
    Symbol.asyncIterator
  ]();
  async function request(method, params) {
    const id = ++count;
    child.stdin.write(
      JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
    );
    const line = await bounded(lines.next(), method);
    assert.equal(line.done, false, err);
    const frame = JSON.parse(line.value);
    assert.equal(frame.id, id);
    return frame;
  }
  try {
    await exercise(request);
    child.stdin.end();
    assert.equal(await bounded(closed, "exit"), exit, err);
    assert.equal(out.trimEnd().split("\n").length, count);
    assert.ok(!out.includes(root) && !out.includes("example.invalid/shared"));
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill();
  }
}
const call = (req) =>
  req("tools/call", { name: "project_context", arguments: {} });
async function trusted(req, binding) {
  assert.equal(
    (await req("initialize", init)).result.serverInfo.name,
    "envlume",
  );
  assert.deepEqual(
    (await req("tools/list", {})).result.tools.map((t) => t.name),
    ["project_context"],
  );
  const report = (await call(req)).result.structuredContent;
  assert.equal(report.result.workspace, binding.alias);
  assert.deepEqual(report.result.capabilities, ["project.inspect"]);
  assert.deepEqual(report.result.targets, []);
}
async function refused(path, id, code) {
  await session(
    path,
    id,
    async (req) =>
      assert.equal((await req("initialize", init)).error.data.errorCode, code),
    3,
  );
}
await refused(clone, a.bindingId, "PROJECT_NOT_TRUSTED");
await refused(
  primary,
  "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  "PROJECT_NOT_TRUSTED",
);
await session(primary, a.bindingId, async (req) => {
  await trusted(req, a);
  cli("project", "revoke", "--binding", a.bindingId);
  assert.equal(
    (await call(req)).result.structuredContent.errorCode,
    "BINDING_REVOKED",
  );
  assert.equal(
    (await req("tools/list", {})).error.data.errorCode,
    "BINDING_REVOKED",
  );
});
await refused(primary, a.bindingId, "BINDING_REVOKED");
await session(tree, worktree.binding.bindingId, async (req) => {
  await trusted(req, worktree.binding);
  confirm("project", "delete", "--project", a.projectId);
  assert.equal((await call(req)).result.structuredContent.ok, false);
});
assert.equal(
  cli("project", "list").projects.some((p) => p.id === a.projectId),
  false,
);
await session(clone, other.bindingId, (req) => trusted(req, other));

const physical = join(root, "physical"),
  replacement = join(root, "replacement"),
  alias = join(root, "selected-link");
mkdirSync(physical);
mkdirSync(replacement);
link(alias, physical);
const linked = add(alias).binding;
await session(physical, linked.bindingId, async (req) => {
  await trusted(req, linked);
  unlink(alias);
  link(alias, replacement);
  assert.equal(
    (await call(req)).result.structuredContent.errorCode,
    "PROJECT_NOT_TRUSTED",
  );
});
assert.equal(cli("project", "inspect", "--path", alias).binding.state, "STALE");
const rebound = add(alias).binding;
assert.notEqual(rebound.bindingId, linked.bindingId);
unlink(alias);
link(alias, physical);
await refused(physical, linked.bindingId, "PROJECT_NOT_TRUSTED");
console.log(
  "STDIO evidence: project.inspect only; mismatched origin and unknown ID denied; live revoke -> BINDING_REVOKED; live delete denied; retarget -> STALE; new registration has a distinct ID.",
);
const moved = join(root, "moved");
renameSync(clone, moved);
assert.equal(cli("project", "inspect", "--path", clone).binding.state, "STALE");
mkdirSync(clone);
await refused(clone, other.bindingId, "PROJECT_NOT_TRUSTED");
const afterMove = add(moved).binding;
assert.notEqual(afterMove.bindingId, other.bindingId);
const escape = primary + "/../Orders";
assert.equal(
  JSON.parse(
    run(exe, ["project", "inspect", "--path", escape, "--json"], root, 2),
  ).errorCode,
  "INVALID_INPUT",
);
const caseAlias = join(root, "MOVED");
if (existsSync(caseAlias)) {
  assert.equal(add(caseAlias).binding.bindingId, afterMove.bindingId);
  console.log(
    "Native case-insensitive path alias resolved to the same binding",
  );
} else {
  mkdirSync(caseAlias);
  assert.notEqual(add(caseAlias).binding.bindingId, afterMove.bindingId);
  console.log(
    "Native case-sensitive filesystem kept distinct directories separate",
  );
}
if (windows)
  for (const path of [
    String.raw`\\server.invalid\share\orders`,
    String.raw`\\?\UNC\server.invalid\share\orders`,
  ]) {
    assert.equal(
      JSON.parse(
        run(exe, ["project", "inspect", "--path", path, "--json"], root, 2),
      ).errorCode,
      "INVALID_INPUT",
    );
  }
console.log(
  "T05: same remote, linked worktree, TTY confirmation, production resolver, live revoke/delete, move, link replacement and fresh binding passed",
  windows ? "(junction + UNC)" : "(symlink)",
);
