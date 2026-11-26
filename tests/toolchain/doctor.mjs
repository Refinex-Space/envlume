import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  linkSync,
  mkdirSync,
  writeFileSync,
  renameSync,
} from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
const [exe, helper] = process.argv.slice(2);
const tmp = process.cwd(),
  windows = process.platform === "win32";
const canary = "T06_PRIVATE_SENTINEL";
const env = {
  ...process.env,
  T06_TEST_PASSWORD: canary,
  JAVA_TOOL_OPTIONS: canary,
};
const javaName = (name) => (windows ? name + ".exe" : name);
function file(path, value) {
  writeFileSync(path, value);
}
function jdk(label, v) {
  const home = join(tmp, "tool homes", label);
  mkdirSync(join(home, "bin"), { recursive: true });
  file(
    join(home, "release"),
    `JAVA_VERSION="${v}"\nOS_ARCH="x86_64"\nIMPLEMENTOR="Eclipse Adoptium"\n`,
  );
  for (const name of ["java", "javac"]) {
    const bin = join(home, "bin", javaName(name));
    try {
      linkSync(helper, bin);
    } catch {
      copyFileSync(helper, bin);
    }
    chmodSync(bin, 0o755);
    file(join(home, "bin", name + ".fixture-version"), v);
  }
  return home;
}
const old = jdk("8 offline", "1.8.0_412"),
  modern = jdk("17 offline", "17.0.11"),
  newest = jdk("21 offline", "21.0.4");
Object.assign(env, {
  JAVA_HOME: newest,
  JAVA_HOME_8_X64: old,
  JAVA_HOME_17_X64: modern,
});
const maven = join(tmp, "tool homes", "Maven 3.9");
mkdirSync(join(maven, "bin"), { recursive: true });
mkdirSync(join(maven, "lib"));
const mvn = join(maven, "bin", windows ? "mvn.cmd" : "mvn");
file(mvn, "metadata only");
chmodSync(mvn, 0o755);
file(join(maven, "lib", "maven-core-3.9.12.jar"), "");
env.MAVEN_HOME = maven;
const first = join(tmp, "Java Eight"),
  second = join(tmp, "Java Seventeen");
for (const [root, v] of [
  [first, 8],
  [second, 17],
]) {
  mkdirSync(root);
  file(
    join(root, "pom.xml"),
    `<project><properties><maven.compiler.release>${v}</maven.compiler.release></properties></project>`,
  );
}
file(
  join(first, "package.json"),
  JSON.stringify({ packageManager: "pnpm@10.33.0" }),
);
file(join(first, "package-lock.json"), "{}");
mkdirSync(join(first, ".venv"));
file(join(first, ".venv", "pyvenv.cfg"), "version = 3.13.0\n");
file(join(first, "mise.toml"), `[tools]\njava="8"\n[env]\nTOKEN="${canary}"\n`);
function command(binary, args, cwd, status = 0) {
  const p = spawnSync(binary, args, {
    cwd,
    env,
    encoding: "utf8",
    timeout: 60000,
  });
  assert.equal(p.status, status, p.stderr || String(p.error));
  return p.stdout;
}
const cli = (cwd, ...args) =>
  JSON.parse(command(exe, [...args, "--json"], cwd));
const approve = (cwd, ...args) =>
  command(helper, ["approve", exe, ...args, "--json"], cwd);
function register(cwd) {
  approve(cwd, "project", "add", cwd);
  return cli(cwd, "project", "inspect", "--path", cwd).binding;
}
const a = register(first),
  b = register(second);
const configure = (cwd, binding, name, ...flags) =>
  approve(
    cwd,
    "toolchain",
    "configure",
    "--binding",
    binding.bindingId,
    "--profile",
    name,
    ...flags,
  );
const params = { profile: "default" };
const doctor = (cwd, binding) =>
  cli(cwd, "doctor", "--binding", binding.bindingId);
const get = (report, tool) => report.result.tools.find((t) => t.tool === tool);
const before = JSON.stringify(env);
const host = cli(first, "doctor");
assert.equal(host.tools.find((t) => t.tool === "jdk").version, "21.0.4");
assert.ok(!JSON.stringify(host).includes(canary));
const denied = JSON.parse(
  command(exe, ["doctor", "--binding", a.bindingId, "--json"], first, 3),
);
assert.equal(denied.errorCode, "POLICY_DENIED");
assert.equal(
  JSON.parse(
    command(
      exe,
      [
        "toolchain",
        "configure",
        "--binding",
        a.bindingId,
        "--profile",
        "default",
        "--json",
      ],
      first,
      3,
    ),
  ).errorCode,
  "POLICY_DENIED",
);

configure(
  first,
  a,
  "default",
  "--tool",
  "jdk",
  "8",
  "--path",
  old,
  "--tool",
  "maven",
  "3.9",
  "--path",
  maven,
);
configure(second, b, "default", "--tool", "jdk", "17", "--path", modern);
const eight = doctor(first, a),
  seventeen = doctor(second, b);
assert.equal(get(eight, "jdk").version, "1.8.0_412");
assert.match(get(eight, "jdk").pathSummary, /explicit.*Temurin.*x64.*javac/);
assert.equal(get(seventeen, "jdk").version, "17.0.11");
assert.equal(get(seventeen, "jdk").repair, null);
assert.match(get(eight, "maven").pathSummary, /paired JDK 1\.8/);
assert.match(get(eight, "pnpm").repair, /lockfile/);
assert.match(get(eight, "python").repair, /Recreate .venv/);
assert.ok(
  !JSON.stringify(eight).includes(canary) &&
    !JSON.stringify(eight).includes("tool homes"),
);
mkdirSync(join(first, ".mvn", "wrapper"), { recursive: true });
file(join(first, windows ? "mvnw.cmd" : "mvnw"), "fixture: never execute");
file(
  join(first, ".mvn", "wrapper", "maven-wrapper.properties"),
  "distributionUrl=https://example.invalid/apache-maven-4.0.0-rc-7-bin.zip",
);
configure(first, a, "default", "--tool", "jdk", "17", "--path", modern);
const crossCompile = doctor(first, a);
assert.equal(get(crossCompile, "jdk").repair, null);
assert.equal(get(crossCompile, "maven").repair, null);
configure(first, a, "default", "--mise");
file(join(first, "mise.toml"), '[tools]\njava="21"\n');
const mismatchedBuild = get(doctor(first, a), "maven");
assert.equal(mismatchedBuild.version, "4.0.0-rc-7");
assert.match(mismatchedBuild.repair, /JDK 17/);
assert.match(get(doctor(first, a), "jdk").pathSummary, /approved-mise/);
assert.equal(get(doctor(first, a), "jdk").version, "1.8.0_412");
assert.equal(get(doctor(second, b), "jdk").version, "17.0.11");
configure(first, a, "default", "--tool", "jdk", "8", "--path", old);
const proc = spawn(exe, ["mcp", "--binding", a.bindingId], {
  cwd: first,
  env,
  stdio: "pipe",
});
const close = new Promise((resolve, reject) => {
  proc.once("error", reject);
  proc.once("close", resolve);
});
const lines = createInterface({ input: proc.stdout })[Symbol.asyncIterator]();
let number = 0;
async function within(p) {
  let timer;
  try {
    return await Promise.race([
      p,
      new Promise((_, fail) => {
        timer = setTimeout(
          () => fail(new Error("doctor STDIO timeout")),
          20000,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
async function request(method, params) {
  const id = ++number;
  proc.stdin.write(
    JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
  );
  const line = await within(lines.next());
  assert.equal(line.done, false);
  const frame = JSON.parse(line.value);
  assert.equal(frame.id, id);
  return frame;
}
try {
  await request("initialize", {
    protocolVersion: "2025-11-25",
    clientInfo: { name: "t06", version: "1" },
    capabilities: {},
  });
  const listed = (await request("tools/list", {})).result.tools;
  assert.deepEqual(
    listed.map((t) => t.name),
    ["project_context", "toolchain_doctor"],
  );
  const call = (args) =>
    request("tools/call", { name: "toolchain_doctor", arguments: args });
  const real = (await call(params)).result.structuredContent;
  assert.equal(get(real, "jdk").version, get(eight, "jdk").version);
  assert.equal(
    (await call({ ...params, path: old })).result.structuredContent.errorCode,
    "INVALID_INPUT",
  );
  renameSync(old, old + " removed");
  const missing = (await call(params)).result.structuredContent;
  assert.equal(get(missing, "jdk").version, null);
  assert.match(get(missing, "jdk").repair, /JDK Home/);
  cli(first, "project", "revoke", "--binding", a.bindingId);
  assert.equal(
    (await call(params)).result.structuredContent.errorCode,
    "BINDING_REVOKED",
  );
  proc.stdin.end();
  assert.equal(await within(close), 0);
} finally {
  if (proc.exitCode === null && proc.signalCode === null) proc.kill();
}
assert.equal(JSON.stringify(env), before);
console.log(
  "T06: offline native fixture probes selected JDK 8 and 17 independently with host JDK 21; Maven pairing, saved mise pins, CLI/MCP diagnostics, redaction and revocation passed.",
);

// Windows hosted images publish these three installed JDKs. Compare the real
// tools on the installed-CLI pass, separately from the deterministic fixtures.
if (
  windows &&
  process.env.T05_NATIVE_CLI &&
  process.env.GITHUB_ACTIONS === "true"
) {
  const homes = [8, 17, 21].map((n) => {
    const home = process.env[`JAVA_HOME_${n}_X64`];
    assert.ok(home, `Missing hosted JDK ${n}`);
    return home;
  });
  env.JAVA_HOME = homes[2];
  const realA = register(first);
  configure(first, realA, "default", "--tool", "jdk", "8", "--path", homes[0]);
  configure(second, b, "default", "--tool", "jdk", "17", "--path", homes[1]);
  const read = (p, binding) => get(doctor(p, binding), "jdk");
  const j8 = read(first, realA),
    j17 = read(second, b);
  const j21 = cli(second, "doctor").tools.find((t) => t.tool === "jdk");
  assert.equal(j8.repair, null);
  assert.equal(j17.repair, null);
  assert.match(j8.version, /^(1\.8\.|8\.)/);
  assert.match(j17.version, /^17\./);
  assert.equal(j21.status, "available");
  assert.match(j21.version, /^21\./);
  console.log(
    `Installed JDKs: first=${j8.version}, second=${j17.version}, host=${j21.version}; java and javac verified.`,
  );
}
