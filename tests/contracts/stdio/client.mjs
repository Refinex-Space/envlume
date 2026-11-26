import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { DatabaseSync } from "node:sqlite";

const [binary, fixture, binding] = process.argv.slice(2);
const load = (file) =>
  JSON.parse(readFileSync(new URL(file, import.meta.url), "utf8"));
const expected = load("../examples.json").project_context.result;
const schema = load("../tools.json").find(
  (t) => t.name === "project_context",
).inputSchema;
const legacy = "2025-11-25";
const modern = "2026-07-28";
const marker = "TEST_ONLY_PRIVATE_INPUT";
const transcript = [];
const record = (stream, message) => transcript.push({ stream, message });
const clientInfo = { name: "envlume-contract", version: "1" };
const meta = {
  "io.modelcontextprotocol/protocolVersion": modern,
  "io.modelcontextprotocol/clientInfo": clientInfo,
  "io.modelcontextprotocol/clientCapabilities": {},
};
const init = { protocolVersion: legacy, clientInfo, capabilities: {} };

async function bounded(promise, label) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("Timeout: " + label)), 15000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function session(exe, id, exercise, exitCode) {
  record("process", {
    program: exe === binary ? "envlume" : "fixture-cli",
    args: ["mcp", "--binding", id],
  });
  const proc = spawn(exe, ["mcp", "--binding", id], { stdio: "pipe" });
  const closed = new Promise((resolve, reject) => {
    proc.once("error", reject);
    proc.once("close", (code, signal) => resolve({ code, signal }));
  });
  let stdout = "",
    stderr = "",
    requests = 0,
    responses = 0;
  proc.stdout.setEncoding("utf8").on("data", (s) => {
    stdout += s;
  });
  proc.stderr.setEncoding("utf8").on("data", (s) => {
    stderr += s;
  });
  const lines = createInterface({ input: proc.stdout, crlfDelay: Infinity })[
    Symbol.asyncIterator
  ]();
  function send(frame) {
    const message = { jsonrpc: "2.0", ...frame };
    record("stdin", message);
    proc.stdin.write(JSON.stringify(message) + "\n");
  }
  async function request(method, params) {
    const id = ++requests;
    send({ id, method, params });
    const line = await bounded(lines.next(), method);
    assert.equal(line.done, false, stderr);
    const frame = JSON.parse(line.value);
    record("stdout", frame);
    assert.equal(frame.id, id);
    assert.equal(frame.jsonrpc, "2.0");
    assert.notEqual("error" in frame, "result" in frame);
    responses++;
    return frame;
  }
  try {
    await exercise({ request, send, pid: proc.pid });
    proc.stdin.end();
    assert.deepEqual(
      await bounded(closed, "exit on EOF"),
      { code: exitCode, signal: null },
      stderr,
    );
    // Check the entire stream, including trailing bytes and unsolicited frames.
    const frames = stdout.split("\n");
    assert.equal(frames.pop(), "");
    assert.equal(frames.length, responses);
    for (const frame of frames) assert.equal(JSON.parse(frame).jsonrpc, "2.0");
    assert.ok(stderr.includes("Envlume MCP: starting STDIO"));
    assert.ok(
      !stderr.includes(marker) && !stderr.includes(process.cwd()),
      stderr,
    );
    assert.ok(!stdout.includes(marker));
    for (const line of stderr.trimEnd().split(/\r?\n/)) record("stderr", line);
    record("exit", exitCode);
    return stderr;
  } finally {
    if (proc.exitCode === null && proc.signalCode === null) proc.kill();
  }
}

function localProcessOnly(pid) {
  if (process.platform === "win32") {
    const sockets = spawnSync("netstat", ["-ano", "-p", "tcp"], {
      encoding: "utf8",
    });
    assert.equal(sockets.status, 0);
    assert.ok(
      !sockets.stdout
        .split(/\r?\n/)
        .some((line) => line.trim().split(/\s+/).at(-1) === String(pid)),
    );
    const children = spawnSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        // Orphan processes retain their original parent PID after Windows reuses it.
        `$p=Get-CimInstance Win32_Process -Filter 'ProcessId = ${pid}' -ErrorAction Stop; if (!$p) { exit 1 }; Get-CimInstance Win32_Process -Filter 'ParentProcessId = ${pid}' -ErrorAction Stop | Where-Object { $_.CreationDate -ge $p.CreationDate } | ForEach-Object { $_.Name }`,
      ],
      { encoding: "utf8" },
    );
    assert.equal(children.status, 0);
    assert.equal(children.stdout.trim(), "", "MCP server spawned a child process: " + children.stdout);
  } else {
    const children = spawnSync("pgrep", ["-P", String(pid)], {
      encoding: "utf8",
    });
    assert.equal(children.status, 1, children.stdout + children.stderr);
    const sockets = spawnSync("lsof", ["-nP", "-a", "-p", String(pid), "-i"], {
      encoding: "utf8",
    });
    assert.equal(sockets.status, 1, sockets.stdout + sockets.stderr);
    assert.equal(sockets.stdout, "");
  }
}

const params = (name, args) => ({ name, arguments: args });
function checkSummary(response) {
  const report = response.result.structuredContent;
  assert.notEqual(response.result.isError, true);
  assert.equal(report.ok, true);
  assert.equal(report.schemaVersion, 1);
  assert.equal(report.capability, "project.inspect");
  assert.match(report.requestId, /^[a-f0-9-]{36}$/);
  assert.deepEqual(report.result, expected);
  return report;
}
await session(
  fixture,
  binding,
  async ({ request, send, pid }) => {
    const handshake = await request("initialize", init);
    assert.equal(handshake.result.protocolVersion, legacy);
    assert.equal(handshake.result.serverInfo.name, "envlume");
    send({ method: "notifications/initialized" });
    const list = await request("tools/list", {});
    assert.deepEqual(
      list.result.tools.map((t) => t.name),
      ["project_context"],
    );
    assert.deepEqual(list.result.tools[0].inputSchema, schema);
    checkSummary(await request("tools/call", params("project_context", {})));
    for (const extra of ["projectId", "bindingId", "cwd"]) {
      const invalid = await request(
        "tools/call",
        params("project_context", { [extra]: marker }),
      );
      assert.equal(invalid.result.isError, true);
      assert.equal(invalid.result.structuredContent.errorCode, "INVALID_INPUT");
    }
    assert.ok((await request("tools/call", params("task_run", {}))).error);
    localProcessOnly(pid);
  },
  0,
);

await session(
  fixture,
  binding,
  async ({ request }) => {
    const discover = await request("server/discover", { _meta: meta });
    assert.ok(discover.result.supportedVersions.includes(modern));
    const unsupported = await request("tools/list", {
      _meta: {
        ...meta,
        "io.modelcontextprotocol/protocolVersion": "2999-01-01",
      },
    });
    assert.equal(unsupported.error.code, -32022);
    assert.ok(unsupported.error.data.supported.includes(modern));
    const list = await request("tools/list", { _meta: meta });
    assert.deepEqual(
      list.result.tools.map((t) => t.name),
      ["project_context"],
    );
    checkSummary(
      await request("tools/call", {
        ...params("project_context", {}),
        _meta: meta,
      }),
    );
    const db = new DatabaseSync("state/data/envlume.db");
    try {
      db.exec(
        "UPDATE capability_grants SET revoked_at=1700000000001 WHERE capability='project.inspect'",
      );
    } finally {
      db.close();
    }
    assert.deepEqual(
      (await request("tools/list", { _meta: meta })).result.tools,
      [],
    );
    const denied = await request("tools/call", {
      ...params("project_context", {}),
      _meta: meta,
    });
    assert.equal(denied.result.structuredContent.errorCode, "POLICY_DENIED");
  },
  0,
);

for (const exe of [binary, fixture]) {
  await session(
    exe,
    "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    async ({ request }) => {
      const refused = await request("server/discover", { _meta: meta });
      assert.equal(refused.error?.data.errorCode, "PROJECT_NOT_TRUSTED");
    },
    3,
  );
  const errorLog = await session(
    exe,
    "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    async ({ request }) => {
      const refused = await request("initialize", init);
      assert.equal(refused.error.data.errorCode, "PROJECT_NOT_TRUSTED");
    },
    3,
  );
  assert.ok(errorLog.includes("handshake failed"));
}
const log = await session(
  fixture,
  binding,
  async ({ request, send }) => {
    const bad = await request("tools/list", {
      _meta: { "io.modelcontextprotocol/clientInfo": marker },
    });
    assert.ok(bad.error);
    send({
      method: "notifications/initialized",
      params: { _meta: { note: marker } },
    });
  },
  2,
);
assert.ok(log.includes("handshake failed"));
if (process.env.ENVLUME_TEST_TRANSCRIPT) {
  writeFileSync(
    process.env.ENVLUME_TEST_TRANSCRIPT,
    transcript.map((event) => JSON.stringify(event)).join("\n") + "\n",
    { flag: "wx" },
  );
}
console.log(
  "T04: both STDIO lifecycles, complete frames, revocation, isolated fixture, process/ports and shutdown passed.",
);
