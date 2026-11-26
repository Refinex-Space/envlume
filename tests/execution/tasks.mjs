import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  mkdirSync,
  copyFileSync,
  writeFileSync,
  readFileSync,
  existsSync,
  readdirSync,
  unlinkSync,
  chmodSync,
} from "node:fs";
import { join } from "node:path";
import Ajv2020 from "ajv/dist/2020.js";
const schema = (name) => JSON.parse(readFileSync(new URL("../contracts/" + name, import.meta.url)));
const ajv = new Ajv2020({ allowUnionTypes: true });
const envelope = ajv.compile(schema("report.schema.json"));
const payloads = Object.fromEntries(schema("tools.json")
  .filter(({name}) => ["task_list", "task_run"].includes(name))
  .map(({name, resultSchema}) => [name, ajv.compile(resultSchema)]));
function report(output, operation) {
  const value = JSON.parse(output);
  assert.ok(envelope(value), JSON.stringify(envelope.errors));
  assert.equal(value.operation, operation);
  if (value.ok) assert.ok(payloads[operation](value.result), JSON.stringify(payloads[operation].errors));
  return value;
}
const [exe, helper] = process.argv.slice(2);
const root = join(process.cwd(), "T07 空格 build");
mkdirSync(root);
const mark = "T07_PRIVATE_CREDENTIAL_SENTINEL";
const env = {
  ...process.env,
  T07_DB_PASSWORD: mark,
  GITHUB_TOKEN: mark,
  SSH_AUTH_SOCK: mark,
  NODE_OPTIONS: "--no-warnings",
  JAVA_TOOL_OPTIONS: "-DT07.inherited=" + mark,
};
const before = JSON.stringify(process.env);
function run(
  program,
  args,
  { code = 0, extra = {}, cwd = root, input, timeout = 210000 } = {},
) {
  const r = spawnSync(program, args, {
    cwd,
    env: { ...env, ...extra },
    input,
    encoding: "utf8",
    timeout,
    maxBuffer: 1024 * 1024,
  });
  assert.equal(
    r.status,
    code,
    JSON.stringify({
      args,
      code: r.status,
      error: r.error?.message,
      stdout: r.stdout,
      stderr: r.stderr,
    }),
  );
  assert.ok(
    !r.stdout.includes(mark) && !r.stderr.includes(mark),
    "credential appeared in output",
  );
  return r.stdout;
}
const cli = (args, opts) => {
  const out = run(exe, [...args, "--json"], opts);
  return args[0] === "task" && ["list", "run"].includes(args[1])
    ? report(out, "task_" + args[1])
    : JSON.parse(out);
};
const approve = (args, opts) =>
  run(helper, ["approve", exe, ...args, "--json"], opts);
const task = (op, key, id) => ["task", op, key, "--binding", id];
const file = (name, value) => writeFileSync(join(root, name), value);
const result = (name) => readFileSync(join(root, name), "utf8");
file(
  "package.json",
  JSON.stringify({
    name: "envlume-task-fixture",
    private: true,
    scripts: { build: "node build.cjs" },
  }),
);
file("package-lock.json", "{}");
file(
  "build.cjs",
  `
const fs=require('node:fs'),cp=require('node:child_process');
const mode=fs.readFileSync('mode','utf8');
for(const k of ['T07_DB_PASSWORD','GITHUB_TOKEN','SSH_AUTH_SOCK','NODE_OPTIONS','JAVA_TOOL_OPTIONS'])if(process.env[k])throw Error('Unexpected child environment');
fs.writeFileSync('node-result',JSON.stringify({node:process.execPath,version:process.version,selected:process.env.NODE,cwd:process.cwd()}));
if(mode==='fail')process.exit(23);
process.stdout.write('T07_PRIVATE_CREDENTIAL_SENTINEL'+'x'.repeat(70000));
process.stderr.write('T07_PRIVATE_CREDENTIAL_SENTINEL'+'e'.repeat(40000));
if(mode==='hold'){
  fs.writeFileSync('pid-'+process.pid,'running');
  cp.spawn(process.execPath,['descendant.cjs','one'],{stdio:'inherit'});
  setInterval(()=>{},100);
}
`,
);
file(
  "descendant.cjs",
  `
const fs=require('node:fs'),cp=require('node:child_process');fs.writeFileSync('pid-'+process.pid,'running');
if(process.argv[2]==='one')cp.spawn(process.execPath,['descendant.cjs','two'],{stdio:'inherit'});
setInterval(()=>{},100);
`,
);
file("mode", "ok");
const unknown = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
assert.equal(
  cli(task("run", "node-build", unknown), { code: 3 }).errorCode,
  "PROJECT_NOT_TRUSTED",
);
approve(["project", "add", root]);
const id = cli(["project", "inspect", "--path", root]).binding.bindingId;
// An installed-tool fixture records version probes outside the workspace.
const probeRoot = join(process.cwd(), "T07 installed JDK");
mkdirSync(join(probeRoot, "bin"), { recursive: true });
for (const name of ["java", "javac"]) {
  const base = join(probeRoot, "bin", name);
  const file = base + (process.platform === "win32" ? ".exe" : "");
  copyFileSync(helper, file);
  chmodSync(file, 0o755);
  writeFileSync(base + ".fixture-version", "17.0.10");
  writeFileSync(base + ".fixture-observe", "");
}
const nodeProfile = ["toolchain", "configure", "--binding", id, "--profile", "default",
  "--tool", "node", "*", "--path", process.execPath];
approve([...nodeProfile, "--tool", "jdk", "*", "--path", probeRoot]);
assert.equal(
  cli(task("run", "node-build", id), { code: 3 }).errorCode,
  "POLICY_DENIED",
);
for (const args of [
  ["--cwd", "../"],
  ["--", "raw"],
  ["--command", "other"],
]) {
  assert.equal(
    cli([...task("run", "node-build", id), ...args], { code: 2 }).errorCode,
    "INVALID_INPUT",
  );
}
assert.equal(
  cli(task("run", "other", id), { code: 2 }).errorCode,
  "INVALID_INPUT",
);
approve([...task("enable", "node-build", id), "--timeout", "60"]);
assert.equal(
  cli(task("run", "node-build", id), { code: 3, input: "run\n" }).errorCode,
  "POLICY_DENIED",
);
assert.ok(!existsSync(join(root, "node-result")));
for (const name of ["java", "javac"]) assert.ok(!existsSync(join(probeRoot, "bin", name + ".fixture-probed")));
approve(nodeProfile);
assert.deepEqual(cli(["task", "list", "--binding", id]).result.tasks, [
  { taskId: "node-build", timeoutSeconds: 60 },
]);
approve(task("run", "node-build", id));
const success = cli(task("run", "node-build", id));
assert.equal(success.status, "success");
assert.equal(success.result.exitCode, 0);
assert.match(success.result.stdoutSummary, /truncated/);
assert.match(success.result.stderrSummary, /truncated/);
assert.ok(success.summary.includes("node=" + process.versions.node));
assert.ok(success.requestId && success.evidenceId);
const observed = JSON.parse(result("node-result"));
assert.equal(observed.version, process.version);
assert.equal(observed.node, observed.selected);
file("mode", "fail");
const failed = cli(task("run", "node-build", id));
assert.match(failed.summary, /Build failed/);
assert.notEqual(failed.result.exitCode, 0);
file("mode", "hold");
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
async function until(condition, label) {
  const end = Date.now() + 30000;
  while (Date.now() < end) {
    if (condition()) return;
    await sleep(40);
  }
  throw Error("Timed out: " + label);
}
function start() {
  const proc = spawn(exe, [...task("run", "node-build", id), "--json"], {
    cwd: root,
    env,
    stdio: "pipe",
  });
  let out = "",
    err = "";
  proc.stdout.setEncoding("utf8").on("data", (s) => (out += s));
  proc.stderr.setEncoding("utf8").on("data", (s) => (err += s));
  const done = new Promise((resolve, reject) => {
    proc.once("error", reject);
    proc.once("close", (code) => resolve({ code, out, err }));
  });
  return { proc, done };
}
const pids = () =>
  readdirSync(root)
    .filter((f) => /^pid-\d+$/.test(f))
    .map((f) => Number(f.slice(4)));
const alive = (id) => {
  try {
    process.kill(id, 0);
    return true;
  } catch (e) {
    if (e.code === "ESRCH") return false;
    throw e;
  }
};
async function gone() {
  const ids = pids();
  assert.ok(ids.length >= 3);
  await until(() => ids.every((p) => !alive(p)), "descendant cleanup");
  for (const p of ids) unlinkSync(join(root, "pid-" + p));
}
const one = start();
await until(() => pids().length >= 3, "first task");
const two = start();
await until(() => pids().length >= 6, "second task");
assert.equal(
  cli(task("run", "node-build", id), { code: 3 }).errorCode,
  "POLICY_DENIED",
);
cli(task("cancel", "node-build", id));
for (const h of [one, two]) {
  const r = await h.done;
  assert.equal(r.code, 3);
  assert.match(report(r.out, "task_run").summary, /cancelled/);
}
await gone();
const eof = start();
await until(() => pids().length >= 3, "disconnect task");
eof.proc.stdin.end();
const disconnected = await eof.done;
assert.equal(disconnected.code, 3);
assert.match(report(disconnected.out, "task_run").summary, /cancelled/);
await gone();
const lost = start();
await until(() => pids().length >= 3, "dead caller task");
lost.proc.kill("SIGKILL");
await lost.done;
await gone();
file("mode", "ok");
approve([...task("enable", "node-build", id), "--timeout", "3"]);
assert.equal(
  cli(task("run", "node-build", id), { code: 3 }).errorCode,
  "POLICY_DENIED",
);
approve(task("run", "node-build", id));
file("mode", "hold");
const timed = cli(task("run", "node-build", id), { code: 6 });
assert.equal(timed.errorCode, "TIMEOUT");
assert.ok(timed.evidenceId);
await gone();
file("mode", "ok");
assert.equal(cli(task("run", "node-build", id)).status, "success");
cli(task("disable", "node-build", id));
assert.deepEqual(cli(["task", "list", "--binding", id]).result.tasks, []);
assert.equal(
  cli(task("run", "node-build", id), { code: 3 }).errorCode,
  "POLICY_DENIED",
);
console.log(
  "T07: Node build, protected environment, failed build, bounded output, two slots, cancel, EOF, killed caller and timeout passed.",
);

// A real JUnit test under Maven's test lifecycle verifies the selected JVM and environment.
file(
  "pom.xml",
  `<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>
<groupId>local.fixture</groupId><artifactId>task-test</artifactId><version>1</version>
<properties><maven.compiler.source>8</maven.compiler.source><maven.compiler.target>8</maven.compiler.target></properties>
<dependencies><dependency><groupId>junit</groupId><artifactId>junit</artifactId><version>4.13.2</version><scope>test</scope></dependency></dependencies>
<build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin><plugin><artifactId>maven-surefire-plugin</artifactId><version>3.5.4</version></plugin></plugins></build></project>`,
);
mkdirSync(join(root, "src/test/java"), { recursive: true });
file(
  "src/test/java/EnvironmentTest.java",
  `import org.junit.Test;
import java.io.File;
import static org.junit.Assert.*;
public class EnvironmentTest {
 @Test public void usesSelectedJdk() throws Exception {
    assertNull(System.getenv("T07_DB_PASSWORD")); assertNull(System.getenv("SSH_AUTH_SOCK"));
    assertNull(System.getProperty("T07.inherited"));
    File selected=new File(System.getenv("JAVA_HOME")).getCanonicalFile();
    File actual=new File(System.getProperty("java.home")).getCanonicalFile();
    assertTrue(selected.equals(actual) || selected.equals(actual.getParentFile()));
 }
}`,
);
const jdk =
  process.env.JAVA_HOME ||
  process.env.JAVA_HOME_17_X64 ||
  process.env.JAVA_HOME_17_ARM64;
assert.ok(jdk, "native Maven test requires a locally installed JDK");
approve([
  "toolchain",
  "configure",
  "--binding",
  id,
  "--profile",
  "default",
  "--tool",
  "jdk",
  "*",
  "--path",
  jdk,
  "--tool",
  "maven",
  "*",
]);
approve(task("enable", "maven-test", id));
approve(task("run", "maven-test", id));
assert.match(
  result("target/surefire-reports/EnvironmentTest.txt"),
  /Tests run: 1, Failures: 0, Errors: 0/,
);
const mvn = cli(task("run", "maven-test", id));
assert.equal(mvn.result.exitCode, 0);
assert.equal(mvn.status, "success");
assert.match(mvn.summary, /maven=\d+\.\d+/);
assert.match(mvn.summary, /jdk=\d+/);
console.log(
  "T07: real Maven",
  mvn.summary,
  "with one Surefire assertion group.",
);

// The wrapper is controlled repository code. Test URL approval without connecting anywhere.
mkdirSync(join(root, ".mvn/wrapper"), { recursive: true });
const wrapper = process.platform === "win32" ? "mvnw.cmd" : "mvnw";
file(
  wrapper,
  process.platform === "win32"
    ? "@echo off\r\necho approved> wrapper-ran\r\n"
    : "#!/bin/sh\nprintf approved > wrapper-ran\n",
);
chmodSync(join(root, wrapper), 0o755);
const url = "https://example.invalid/apache-maven-3.9.12-bin.zip";
file(".mvn/wrapper/maven-wrapper.properties", "distributionUrl=" + url);
approve([
  "toolchain",
  "configure",
  "--binding",
  id,
  "--profile",
  "default",
  "--tool",
  "jdk",
  "*",
  "--path",
  jdk,
]);
assert.equal(
  cli(task("run", "maven-test", id), { code: 3 }).errorCode,
  "POLICY_DENIED",
);
assert.ok(!existsSync(join(root, "wrapper-ran")));
approve(task("run", "maven-test", id), { extra: { T07_EXPECT_DOWNLOAD: url } });
assert.ok(existsSync(join(root, "wrapper-ran")));
unlinkSync(join(root, "wrapper-ran"));
file(
  ".mvn/wrapper/maven-wrapper.properties",
  "distributionUrl=https://example.invalid/new/apache-maven-3.9.12-bin.zip",
);
assert.equal(
  cli(task("run", "maven-test", id), { code: 3 }).errorCode,
  "POLICY_DENIED",
);
assert.ok(!existsSync(join(root, "wrapper-ran")));
assert.equal(JSON.stringify(process.env), before);
console.log(
  "T07: wrapper approval pinned to script, profile and distribution URL; changed URL rejected.",
);
