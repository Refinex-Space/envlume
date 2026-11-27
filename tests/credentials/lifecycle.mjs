// Native provider, with only test-generated logins submitted through a private PTY.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
const [cli, helper] = process.argv.slice(2);
const cwd = join(process.cwd(), "credential fixture");
mkdirSync(cwd);
function call(args, code = 0, confirm = false) {
	const r = spawnSync(
		confirm ? helper : cli,
		[...(confirm ? ["approve", cli] : []), ...args, "--json"],
		{ cwd, encoding: "utf8", timeout: 90000 },
	);
	for (const secret of ["CLI_T10_fixture_user", "CLI_T10_fixture_password"]) {
		assert.ok(
			!r.stdout?.includes(secret) && !r.stderr?.includes(secret),
			"secret in process output",
		);
	}
	assert.equal(
		r.status,
		code,
		"Unexpected exit in " + args[0] + " " + args[1] + " " + (r.stderr ?? ""),
	);
	if (!confirm) return JSON.parse(r.stdout);
}
async function checkAgent(scope) {
	const proc = spawn(cli, ["mcp", ...scope], { cwd, stdio: "pipe" });
	const closed = new Promise((resolve, reject) => {
		proc.once("error", reject);
		proc.once("close", resolve);
	});
	const lines = createInterface({ input: proc.stdout })[Symbol.asyncIterator]();
	let stderr = "",
		seq = 0;
	proc.stderr.setEncoding("utf8").on("data", (s) => (stderr += s));
	async function request(method, params) {
		const id = ++seq;
		proc.stdin.write(
			JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
		);
		let timer;
		try {
			const line = await Promise.race([
				lines.next(),
				new Promise((_, reject) => {
					timer = setTimeout(
						() => reject(Error("credential STDIO timeout")),
						30000,
					);
				}),
			]);
			assert.ok(!line.done);
			for (const marker of [
				"CLI_T10_fixture_user",
				"CLI_T10_fixture_password",
				"envlume:v1:",
				"local.invalid",
			])
				assert.ok(!line.value.includes(marker));
			const msg = JSON.parse(line.value);
			assert.equal(msg.id, id);
			return msg;
		} finally {
			clearTimeout(timer);
		}
	}
	try {
		await request("initialize", {
			protocolVersion: "2025-11-25",
			capabilities: {},
			clientInfo: { name: "t10-native", version: "1" },
		});
		proc.stdin.write(
			JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) +
				"\n",
		);
		const context = await request("tools/call", {
			name: "project_context",
			arguments: {},
		});
		assert.ok(context.result.structuredContent.ok);
		const tools = (await request("tools/list", {})).result.tools;
		assert.ok(tools.some((t) => t.name === "environment_list"));
		assert.ok(!tools.some((t) => /secret|credential|password/i.test(t.name)));
		assert.ok(
			(
				await request("tools/call", {
					name: "get_secret",
					arguments: { target: "dev", resource: "db" },
				})
			).error,
		);
		const view = (
			await request("tools/call", { name: "environment_list", arguments: {} })
		).result.structuredContent;
		assert.deepEqual(view.result.environments, [
			{ target: "dev", resources: [{ alias: "db", health: "unknown" }] },
		]);
	} finally {
		proc.stdin.end();
		await closed;
	}
	for (const marker of ["CLI_T10_fixture_user", "CLI_T10_fixture_password"])
		assert.ok(!stderr.includes(marker));
}
call(["project", "add", cwd], 0, true);
const id = call(["project", "inspect", "--path", cwd]).binding.bindingId;
const scope = ["--binding", id];
call(["environment", "set", "dev", ...scope], 0, true);
call(
	[
		"environment",
		"resource",
		"dev",
		"db",
		"local.invalid",
		"3306",
		"VERIFY_IDENTITY",
		"fixture",
		...scope,
	],
	0,
	true,
);
call(
	[
		"environment",
		"resource",
		"dev",
		"shared",
		"local.invalid",
		"3306",
		"VERIFY_IDENTITY",
		"other",
		...scope,
	],
	0,
	true,
);
call(
	["environment", "grant", "dev", "db", "resource.connect", ...scope],
	0,
	true,
);
const cmd = (op) => ["credential", op, "dev", "db", ...scope];
assert.equal(call(cmd("status")).state, "NOT_SET");
assert.equal(call(cmd("set"), 3).state, "ACCESS_DENIED");
try {
	call(cmd("set"), 0, true);
	assert.equal(call(cmd("status")).state, "AVAILABLE");
	await checkAgent(scope);
	call(["credential", "rebind", "dev", "shared", "db", ...scope], 0, true);
	assert.equal(
		call(["credential", "status", "dev", "shared", ...scope]).state,
		"AVAILABLE",
	);
	call(cmd("replace"), 0, true);
	assert.equal(call(cmd("status")).state, "AVAILABLE");
} finally {
	call(cmd("clear"), 0, true);
	call(["credential", "clear", "dev", "shared", ...scope], 0, true);
}
assert.equal(call(cmd("status")).state, "NOT_SET");
call(["credential", "cleanup", ...scope], 0, true);
console.log(
	"T10 CLI: terminal-only submission, status, rotation and cleanup passed without exposing the test login.",
);
