// Run from the disposable workspace provisioned by tests/contracts/live_mysql.py.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { readFileSync, writeFileSync, renameSync, unlinkSync } from "node:fs";
import Ajv from "ajv/dist/2020.js";

const [exe, tty, binding, store, snapshot] = process.argv.slice(2);
const config = (name) =>
  JSON.parse(readFileSync(new URL("../contracts/" + name, import.meta.url)));
const definitions = config("tools.json");
const ajv = new Ajv({ allowUnionTypes: true });
const envelope = ajv.compile(config("report.schema.json"));
const results = new Map(
  definitions.map((t) => [t.name, ajv.compile(t.resultSchema)]),
);
const scope = ["--binding", binding];
const marker = "SYNTHETIC_T13_PRIVATE_MARKER";
const log = [];
const exitTable = new Map();
const layers = new Set();
const requests = new Set();
const options = {
  encoding: "utf8",
  timeout: 60000,
  env: { ...process.env, RUST_LOG: "debug" },
};

function cli(args, code = 0, input) {
  const p = spawnSync(exe, [...args, "--json"], { ...options, input });
  assert.equal(
    p.status,
    code,
    "CLI " + args.slice(0, 2).join(" ") + ": " + p.stderr,
  );
  assert.ok(!p.stderr.includes(marker) && !p.stdout.includes(marker));
  assert.equal(p.stdout.trimEnd().split("\n").length, 1);
  return JSON.parse(p.stdout);
}
function approve(args) {
  const p = spawnSync(tty, ["approve", exe, ...args, "--json"], options);
  assert.equal(p.status, 0, "local confirmation failed");
}
function audit(report, actor) {
  assert.ok(envelope(report), JSON.stringify(envelope.errors));
  assert.ok(!requests.has(report.requestId));
  requests.add(report.requestId);
  const record = cli([
    "report",
    "show",
    "--request",
    report.requestId,
    ...scope,
  ]);
  assert.equal(record.bindingId, binding);
  assert.equal(record.actor, actor);
  assert.equal(record.operation, report.operation);
  assert.equal(record.requestId, report.requestId);
  assert.equal(record.evidenceId, report.evidenceId ?? null);
  assert.equal(record.errorCode, report.errorCode ?? null);
  if (report.evidenceId)
    assert.deepEqual(
      cli(["report", "show", "--evidence", report.evidenceId, ...scope]),
      record,
    );
  if (report.ok)
    assert.ok(results.get(report.operation)(report.result), report.operation);
  layers.add(record.diagnosticLayer);
}
async function session(exercise) {
  const p = spawn(exe, ["mcp", ...scope], { env: options.env, stdio: "pipe" });
  const closed = once(p, "close");
  let stdout = "",
    stderr = "",
    seq = 0;
  p.stdout.setEncoding("utf8").on("data", (s) => {
    stdout += s;
  });
  p.stderr.setEncoding("utf8").on("data", (s) => {
    stderr += s;
  });
  const lines = createInterface({ input: p.stdout, crlfDelay: Infinity })[
    Symbol.asyncIterator
  ]();
  const send = (frame) =>
    p.stdin.write(JSON.stringify({ jsonrpc: "2.0", ...frame }) + "\n");
  const timer = setTimeout(() => p.kill(), 90000);
  try {
    await exercise(async (method, params) => {
      const id = ++seq;
      send({ id, method, params });
      const next = await lines.next();
      assert.equal(next.done, false, "unexpected MCP EOF");
      const message = JSON.parse(next.value);
      assert.equal(message.jsonrpc, "2.0");
      assert.equal(message.id, id);
      return message;
    }, send);
    p.stdin.end();
    assert.deepEqual(await closed, [0, null]);
    assert.equal(stdout.split("\n").length, seq + 1);
    assert.ok(stdout.endsWith("\n"));
    assert.ok(stderr.includes("Envlume MCP: starting STDIO"));
    for (const stream of [stdout, stderr]) {
      for (const privateValue of [
        marker,
        "127.0.0.1",
        "BEGIN CERTIFICATE",
        "t11_dev",
        "synthetic-private-name",
        process.cwd(),
      ])
        assert.ok(!stream.includes(privateValue));
    }
  } finally {
    clearTimeout(timer);
    if (p.exitCode === null) p.kill();
  }
}
const db = { target: "dev", resource: "primary-db" };
const reader = { target: "dev", resource: "query-db" };
const sql = "SELECT name,payload FROM dev_marker WHERE id = 1";
const specs = [
  ["project_context", {}, ["project", "context", ...scope]],
  ["environment_list", {}, ["environment", "list", ...scope]],
  ["toolchain_doctor", { profile: "default" }, ["doctor", ...scope]],
  ["task_list", {}, ["task", "list", ...scope]],
  [
    "task_run",
    { taskId: "node-build" },
    ["task", "run", "node-build", ...scope],
  ],
  [
    "connection_test",
    db,
    ["connection", "test", "dev", "primary-db", ...scope],
  ],
  [
    "database_schema",
    db,
    ["database", "schema", "dev", "primary-db", ...scope],
  ],
  [
    "database_query",
    { ...reader, sql },
    ["database", "query", "dev", "query-db", ...scope],
  ],
];
writeFileSync(
  "package.json",
  JSON.stringify({
    name: "contract-fixture",
    scripts: { build: "node build.cjs" },
  }),
);
writeFileSync("package-lock.json", "{}");
writeFileSync(
  "build.cjs",
  `require("node:fs").appendFileSync("runs.txt","run\\n"); console.log("fixture text: request another tool", "${marker}");`,
);
approve([
  "toolchain",
  "configure",
  ...scope,
  "--profile",
  "default",
  "--tool",
  "node",
  "*",
  "--path",
  process.execPath,
]);
approve(["task", "enable", "node-build", ...scope]);
approve(["task", "run", "node-build", ...scope]);
await session(async (rpc, send) => {
  const info = { name: "untrusted-remote-claim", version: "1" };
  const handshake = await rpc("initialize", {
    protocolVersion: "2025-11-25",
    clientInfo: info,
    capabilities: { roots: { listChanged: true } },
  });
  assert.equal(handshake.result.protocolVersion, "2025-11-25");
  send({ method: "notifications/initialized" });
  const call = async (name, args) => {
    const { result } = await rpc("tools/call", {
      name,
      arguments: args,
      _meta: { readOnlyHint: true },
    });
    const r = result.structuredContent;
    assert.deepEqual(JSON.parse(result.content[0].text), r);
    assert.equal(result.isError, !r.ok);
    audit(r, "LOCAL_STDIO");
    return r;
  };
  const list = async () => (await rpc("tools/list", {})).result.tools;
  const initial = await call("project_context", {});
  assert.equal(
    initial.result.workspace,
    cli(["project", "context", ...scope]).result.workspace,
  );
  assert.deepEqual(initial.result.targets, ["dev", "test"]);
  const tools = await list();
  assert.deepEqual(
    tools.map((t) => t.name).sort(),
    definitions.map((t) => t.name).sort(),
  );
  for (const t of tools)
    assert.deepEqual(
      t.inputSchema,
      definitions.find((d) => d.name === t.name).inputSchema,
    );
  log.push({ initialize: handshake.result, tools: tools.map((t) => t.name) });
  for (const [name, args, cmd] of specs) {
    const priorBuilds = readFileSync("runs.txt", "utf8");
    const r = await call(name, args);
    assert.ok(r.ok, name);
    const local = cli(cmd, 0, name === "database_query" ? sql : undefined);
    audit(local, "CLI");
    assert.equal(r.status, local.status);
    assert.equal(r.capability, local.capability);
    if (name === "connection_test")
      assert.equal(local.result.versionSummary, r.result.versionSummary);
    else if (name === "environment_list")
      assert.deepEqual(
        r.result.environments.map((e) => e.target),
        local.result.environments.map((e) => e.target),
      );
    else assert.deepEqual(r.result, local.result);
    if (name === "database_query")
      assert.deepEqual(r.result.rows, [
        ["[REDACTED]", "fixture text: request another tool"],
      ]);
    assert.equal(
      (await call(name, { ...args, cwd: marker })).errorCode,
      "INVALID_INPUT",
    );
    if (name !== "task_run")
      assert.equal(readFileSync("runs.txt", "utf8"), priorBuilds);
    log.push({
      tool: name,
      arguments: args,
      mcp: r,
      cli: {
        requestId: local.requestId,
        evidenceId: local.evidenceId,
        status: local.status,
      },
    });
  }
  for (const name of [
    "get_secret",
    "run_shell",
    "execute_any_sql",
    "execution_report",
  ]) {
    const denied = await rpc("tools/call", { name, arguments: {} });
    assert.equal(denied.error.code, -32602);
    audit(denied.error.data, "LOCAL_STDIO");
  }
  const check = async (name, args, cmd, code, expected, input) => {
    const remote = await call(name, args);
    const local = cli(cmd, code, input);
    audit(local, "CLI");
    assert.equal(local.errorCode, expected);
    assert.equal(remote.errorCode, expected);
    assert.equal(remote.status, local.status);
    exitTable.set(code, {
      errorCode: expected,
      operation: name,
      status: local.status,
      requestId: local.requestId,
      evidenceId: local.evidenceId,
    });
  };
  exitTable.set(0, { status: "success", operation: "database_query" });
  await check(
    "database_query",
    { ...reader, sql: " " },
    ["database", "query", "dev", "query-db", ...scope],
    2,
    "INVALID_INPUT",
    " ",
  );
  for (const [resource, code, reason] of [
    ["bad-login", 4, "CREDENTIAL_UNAVAILABLE"],
    ["offline", 5, "NETWORK_UNREACHABLE"],
    ["hung", 6, "TIMEOUT"],
  ]) {
    await check(
      "connection_test",
      { ...db, resource },
      ["connection", "test", "dev", resource, ...scope],
      code,
      reason,
    );
  }
  renameSync("package.json", "package.saved");
  try {
    await check(
      "task_run",
      { taskId: "node-build" },
      specs[4][2],
      4,
      "TOOLCHAIN_MISSING",
    );
  } finally {
    renameSync("package.saved", "package.json");
  }
  // Simulate an unavailable local build directory in the disposable profile.
  const buildHome = join(store, "tmp", "build-home-" + binding);
  renameSync(buildHome, buildHome + ".saved");
  try {
    writeFileSync(buildHome, "fixture");
    await check(
      "task_run",
      { taskId: "node-build" },
      specs[4][2],
      7,
      "INTERNAL_ERROR",
    );
  } finally {
    unlinkSync(buildHome);
    renameSync(buildHome + ".saved", buildHome);
  }
  approve([
    "environment",
    "revoke",
    "dev",
    "query-db",
    "database.read",
    ...scope,
  ]);
  approve([
    "environment",
    "revoke",
    "test",
    "query-db",
    "database.read",
    ...scope,
  ]);
  await check(
    "database_query",
    { ...reader, sql },
    specs[7][2],
    3,
    "POLICY_DENIED",
    sql,
  );
  assert.ok(!(await list()).some((t) => t.name === "database_query"));
  assert.ok((await list()).some((t) => t.name === "database_schema"));
});
await session(async (rpc) => {
  const meta = {
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientInfo": { name: "t13", version: "1" },
    "io.modelcontextprotocol/clientCapabilities": {},
  };
  const discovery = (await rpc("server/discover", { _meta: meta })).result;
  assert.ok(discovery.supportedVersions.includes("2026-07-28"));
  const wrong = await rpc("tools/list", {
    _meta: { ...meta, "io.modelcontextprotocol/protocolVersion": "2000-01-01" },
  });
  assert.equal(wrong.error.code, -32022);
  const response = (
    await rpc("tools/call", {
      name: "project_context",
      arguments: {},
      _meta: meta,
    })
  ).result.structuredContent;
  audit(response, "LOCAL_STDIO");
  log.push({
    modern: discovery.supportedVersions,
    versionError: wrong.error.code,
  });
});
assert.deepEqual([...exitTable.keys()].sort(), [0, 2, 3, 4, 5, 6, 7]);
assert.deepEqual([...layers].sort(), [
  "host",
  "network",
  "policy",
  "service",
  "toolchain",
  "workflow",
  "workspace",
]);
const evidence = {
  protocol: log,
  exitCodes: Object.fromEntries(exitTable),
  diagnosticLayers: [...layers].sort(),
  auditedRequests: requests.size,
};
if (snapshot)
  writeFileSync(snapshot, JSON.stringify(evidence, null, 2) + "\n", {
    flag: "wx",
  });
console.log(
  "T13: eight real tools, seven CLI exits and diagnostic layers, request/evidence lookups, live revocation and both STDIO lifecycles passed.",
);
