// Exercise the same temporary registered binding through the independent CLI and STDIO.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
	renameSync,
	rmSync,
	mkdirSync,
	readFileSync,
	writeFileSync,
	existsSync,
} from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { DatabaseSync } from "node:sqlite";

const [exe, helper] = process.argv.slice(2);
const root = join(process.cwd(), "evidence");
const sibling = join(process.cwd(), "evidence-sibling");
for (const path of [root, sibling]) mkdirSync(path);
const secret = "T08_FIXTURE_PASSWORD_DO_NOT_PERSIST";
const env = { ...process.env, T08_PASSWORD: secret };
function command(args, { cwd = root, code = 0, confirm = false } = {}) {
	const r = spawnSync(
		confirm ? helper : exe,
		[...(confirm ? ["approve", exe] : []), ...args, "--json"],
		{
			cwd,
			env,
			encoding: "utf8",
			timeout: 210000,
		},
	);
	assert.ok(
		!r.stdout?.includes(secret) && !r.stderr?.includes(secret),
		"sentinel in CLI output",
	);
	assert.equal(r.status, code, r.stderr + r.stdout);
	if (!confirm) return JSON.parse(r.stdout);
}
for (const cwd of [root, sibling])
	command(["project", "add", cwd], { cwd, confirm: true });
const binding = (cwd) =>
	command(["project", "inspect", "--path", cwd], { cwd }).binding.bindingId;
const id = binding(root),
	other = binding(sibling);
const flag = ["--binding", id];
const run = ["task", "run", "node-build", ...flag];
const show = (key, identifier) =>
	command(["report", "show", key, identifier, ...flag]);
const request = (r) => show("--request", r.requestId);
const evidence = (r) => show("--evidence", r.evidenceId);
const recordFor = (r, actor, decision) => {
	const record = request(r);
	assert.equal(record.actor, actor);
	assert.equal(record.decision, decision);
	assert.equal(record.bindingId, id);
	assert.equal(record.requestId, r.requestId);
	assert.equal(record.durationMs, r.durationMs);
	assert.equal(record.timestampUtc, r.timestampUtc);
	if (r.evidenceId) assert.deepEqual(evidence(r), record);
	else assert.equal(record.evidenceId, null);
	assert.ok(record.diagnostic);
	return record;
};
writeFileSync(
	join(root, "package.json"),
	JSON.stringify({
		name: "evidence-fixture",
		scripts: { build: "node build.cjs" },
	}),
);
writeFileSync(join(root, "package-lock.json"), "{}");
writeFileSync(
	join(root, "build.cjs"),
	`
const fs=require("node:fs");
if(process.env.T08_PASSWORD) throw new Error("credential inherited");
process.stdout.write("[INFO] Tests run: 7, Failures: 0, Errors: 0, Skipped: 0\\n");
process.stdout.write("${secret}" + "x".repeat(40000));
process.stderr.write("${secret}" + "y".repeat(40000));
if(fs.existsSync("hold")) { fs.writeFileSync("started", "yes"); setInterval(()=>{},100); }
`,
);
command(
	[
		"toolchain",
		"configure",
		...flag,
		"--profile",
		"default",
		"--tool",
		"node",
		"*",
		"--path",
		process.execPath,
	],
	{ confirm: true },
);
const denied = command(run, { code: 3 });
recordFor(denied, "CLI", "DENIED");
const injected = command(["task", "run", secret, ...flag], { code: 2 });
assert.equal(recordFor(injected, "CLI", "DENIED").taskId, null);
const proc = spawn(exe, ["mcp", ...flag], { cwd: root, env, stdio: "pipe" });
let stderr = "";
proc.stderr.setEncoding("utf8").on("data", (s) => (stderr += s));
const closed = new Promise((resolve) => proc.once("close", resolve));
const reader = createInterface({ input: proc.stdout, crlfDelay: Infinity })[
	Symbol.asyncIterator
]();
let sequence = 0;
async function frame(method, params) {
	const id = ++sequence;
	proc.stdin.write(
		JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
	);
	let timer;
	try {
		const line = await Promise.race([
			reader.next(),
			new Promise((_, reject) => {
				timer = setTimeout(
					() => reject(Error("STDIO evidence timeout")),
					45000,
				);
			}),
		]);
		assert.ok(!line.done && !line.value.includes(secret));
		const msg = JSON.parse(line.value);
		assert.equal(msg.id, id);
		assert.ok(!msg.error, JSON.stringify(msg.error));
		return msg.result;
	} finally {
		clearTimeout(timer);
	}
}
const call = async (name, args) =>
	(await frame("tools/call", { name, arguments: args })).structuredContent;
try {
	await frame("initialize", {
		protocolVersion: "2025-11-25",
		clientInfo: { name: "claimed-client", version: "1" },
		capabilities: {},
	});
	const context = await call("project_context", {});
	assert.ok(context.ok);
	assert.ok(
		!(await frame("tools/list", {})).tools.some((t) => t.name === "task_run"),
	);
	recordFor(
		await call("task_run", { taskId: "node-build" }),
		"LOCAL_STDIO",
		"DENIED",
	);
	command(["task", "enable", "node-build", ...flag, "--timeout", "4"], {
		confirm: true,
	});
	const advertised = (await frame("tools/list", {})).tools;
	const contracts = JSON.parse(
		readFileSync(new URL("../contracts/tools.json", import.meta.url), "utf8"),
	);
	for (const name of ["task_list", "task_run"]) {
		assert.deepEqual(
			advertised.find((t) => t.name === name).inputSchema,
			contracts.find((t) => t.name === name).inputSchema,
		);
	}
	const listed = advertised.map((t) => t.name);
	assert.ok(listed.includes("task_list") && listed.includes("task_run"));
	const doctor = await call("toolchain_doctor", { profile: "default" });
	const diagnosis = recordFor(doctor, "LOCAL_STDIO", "SUCCESS");
	assert.equal(diagnosis.diagnosticLayer, "toolchain");
	assert.equal(
		diagnosis.tools.find((t) => t.tool === "node").version,
		process.versions.node,
	);
	const listing = await call("task_list", {});
	recordFor(listing, "LOCAL_STDIO", "SUCCESS");
	assert.equal(listing.result.tasks[0].taskId, "node-build");
	recordFor(
		await call("task_run", { taskId: "node-build" }),
		"LOCAL_STDIO",
		"DENIED",
	);
	const bad = await call("task_run", { taskId: "node-build", cmd: secret });
	assert.equal(bad.errorCode, "INVALID_INPUT");
	recordFor(bad, "LOCAL_STDIO", "DENIED");
	command(run, { confirm: true });
	const result = await call("task_run", { taskId: "node-build" });
	assert.equal(result.result.exitCode, 0);
	const record = recordFor(result, "LOCAL_STDIO", "SUCCESS");
	assert.equal(record.versions.node, process.versions.node);
	assert.equal(record.stdoutSummary.assertions, null);
	assert.ok(record.stdoutSummary.truncated && record.stderrSummary.truncated);
	const cli = command(run);
	recordFor(cli, "CLI", "SUCCESS");
	for (const [key, value] of [
		["--evidence", cli.evidenceId],
		["--request", denied.requestId],
	]) {
		const blocked = command(
			["report", "show", key, value, "--binding", other],
			{ cwd: sibling, code: 3 },
		);
		assert.equal(blocked.errorCode, "POLICY_DENIED");
	}
	writeFileSync(join(root, "hold"), "");
	const timeout = await call("task_run", { taskId: "node-build" });
	assert.equal(timeout.errorCode, "TIMEOUT");
	recordFor(timeout, "LOCAL_STDIO", "FAILED");
	const cliTimeout = command(run, { code: 6 });
	recordFor(cliTimeout, "CLI", "FAILED");
	rmSync(join(root, "started"));
	const cancelling = call("task_run", { taskId: "node-build" });
	const deadline = Date.now() + 20000;
	while (!existsSync(join(root, "started"))) {
		assert.ok(Date.now() < deadline, "MCP build did not start");
		await new Promise((resolve) => setTimeout(resolve, 20));
	}
	command(["task", "cancel", "node-build", ...flag]);
	const cancelled = await cancelling;
	assert.equal(cancelled.ok, false);
	recordFor(cancelled, "LOCAL_STDIO", "SKIPPED");
	rmSync(join(root, "hold"));
	renameSync(join(root, "package.json"), join(root, "package.disabled"));
	const missing = command(run, { code: 4 });
	assert.equal(
		recordFor(missing, "CLI", "SKIPPED").diagnosticLayer,
		"toolchain",
	);
	command(["report", "clear", "--binding", other], { cwd: sibling });
	assert.equal(evidence(cli).requestId, cli.requestId);
	// Inspect only the isolated test catalogue; the production binary uses the runner's own profile.
	const dbFile = join(process.env.T05_FIXTURE_STORE, "data", "envlume.db");
	if (exe === helper) {
		const db = new DatabaseSync(dbFile);
		try {
			const rows = db
				.prepare(
					"SELECT actor,decision,details_json,sanitized_summary FROM execution_records",
				)
				.all();
			assert.ok(rows.length >= 8);
			assert.ok(!JSON.stringify(rows).includes(secret));
			for (const suffix of ["", "-wal"])
				if (existsSync(dbFile + suffix))
					assert.ok(
						!readFileSync(dbFile + suffix).includes(Buffer.from(secret)),
					);
			db.prepare(
				"UPDATE execution_records SET created_at=1 WHERE evidence_id=?",
			).run(Buffer.from(cli.evidenceId.replaceAll("-", ""), "hex"));
			command(["report", "show", "--evidence", cli.evidenceId, ...flag], {
				code: 3,
			});
			assert.equal(
				db
					.prepare(
						"SELECT count(*) n FROM execution_records WHERE created_at=1",
					)
					.get().n,
				0,
			);
		} finally {
			db.close();
		}
	}
	const otherDenied = command(
		["task", "run", "node-build", "--binding", other],
		{ cwd: sibling, code: 3 },
	);
	assert.ok(command(["report", "clear", ...flag]).deleted > 0);
	assert.equal(
		command(
			[
				"report",
				"show",
				"--request",
				otherDenied.requestId,
				"--binding",
				other,
			],
			{ cwd: sibling },
		).decision,
		"DENIED",
	);
	command(["report", "show", "--evidence", result.evidenceId, ...flag], {
		code: 3,
	});
	assert.ok(!stderr.includes(secret));
	console.log(
		"T08-A02/A03: CLI/STDIO requests, decisions, timeout, scoped lookup, retention, redacted streams and cleanup passed.",
	);
} finally {
	proc.stdin.end();
	await closed;
}
