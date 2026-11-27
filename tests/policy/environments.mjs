import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
const [cli, helper] = process.argv.slice(2);
const root = join(process.cwd(), "target policy");
mkdirSync(root);
const invoke = (args, code = 0, confirm = false) => {
	const r = spawnSync(
		confirm ? helper : cli,
		[...(confirm ? ["approve", cli] : []), ...args, "--json"],
		{ cwd: root, encoding: "utf8", timeout: 60000 },
	);
	assert.equal(r.status, code, r.stderr + r.stdout);
	if (!confirm) return JSON.parse(r.stdout);
};
invoke(["project", "add", root], 0, true);
const { bindingId, projectId } = invoke([
	"project",
	"inspect",
	"--path",
	root,
]).binding;
const scope = ["--binding", bindingId];
const edit = (args) => invoke(["environment", ...args, ...scope], 0, true);
const read = () =>
	invoke(["environment", "list", ...scope]).result.environments;
const processMcp = spawn(cli, ["mcp", ...scope], { cwd: root, stdio: "pipe" });
const closed = new Promise((resolve) => processMcp.once("close", resolve));
const lines = createInterface({ input: processMcp.stdout })[
	Symbol.asyncIterator
]();
let seq = 0;
async function request(method, params) {
	const id = ++seq;
	processMcp.stdin.write(
		JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
	);
	let timer;
	try {
		const line = await Promise.race([
			lines.next(),
			new Promise((_, reject) => {
				timer = setTimeout(
					() => reject(Error("MCP environment request timed out")),
					30000,
				);
			}),
		]);
		assert.ok(!line.done);
		assert.ok(!line.value.includes("internal.invalid"));
		const frame = JSON.parse(line.value);
		assert.equal(frame.id, id);
		return frame.result;
	} finally {
		clearTimeout(timer);
	}
}
const call = async (name, args = {}) =>
	(await request("tools/call", { name, arguments: args })).structuredContent;
const tools = async () =>
	(await request("tools/list", {})).tools.map((t) => t.name);
try {
	await request("initialize", {
		protocolVersion: "2025-11-25",
		clientInfo: { name: "fixture", version: "1" },
		capabilities: {},
	});
	assert.ok((await call("project_context")).ok);
	assert.ok(!(await tools()).includes("environment_list"));
	assert.equal(
		invoke(["environment", "set", "dev", "DEV", "LOW", ...scope], 3).errorCode,
		"POLICY_DENIED",
	);
	for (const [alias, kind] of [
		["dev", "DEV"],
		["qa", "TEST"],
		["stage", "STAGING"],
		["live", "PROD"],
	]) {
		edit(["set", alias, kind, "MEDIUM"]);
		edit([
			"resource",
			alias,
			"primary-db",
			alias + ".internal.invalid",
			"3306",
			"VERIFY_IDENTITY",
			"orders",
		]);
	}
	assert.equal((await call("environment_list")).errorCode, "POLICY_DENIED");
	const grant = (target, action) =>
		edit(["grant", target, "primary-db", action]);
	grant("dev", "database.schema");
	grant("qa", "database.read");
	grant("stage", "resource.connect");
	assert.ok((await tools()).includes("environment_list"));
	const before = read();
	assert.deepEqual(
		before.map((e) => e.target),
		["dev", "qa", "stage"],
	);
	assert.deepEqual(
		before.flatMap((e) => e.resources),
		Array(3).fill({ alias: "primary-db", health: "unknown" }),
	);
	assert.deepEqual(
		(await call("environment_list")).result.environments,
		before,
	);
	assert.deepEqual(
		invoke(["environment", "list", "--project", projectId]).result.environments,
		before,
	);
	assert.equal(
		(await call("environment_list", { target: "live" })).errorCode,
		"INVALID_INPUT",
	);
	assert.equal(
		invoke(
			["environment", "grant", "live", "primary-db", "database.read", ...scope],
			3,
		).errorCode,
		"TARGET_NOT_ALLOWED",
	);
	assert.equal(
		invoke(
			[
				"environment",
				"grant",
				"stage",
				"primary-db",
				"database.read",
				...scope,
			],
			3,
		).errorCode,
		"TARGET_NOT_ALLOWED",
	);
	for (const target of ["dev", "qa", "stage", "live"]) {
		for (const cap of [
			"database.write",
			"ssh.exec",
			"redis.write",
			"es.write",
		]) {
			assert.equal(
				invoke(["environment", "grant", target, "primary-db", cap, ...scope], 2)
					.errorCode,
				"INVALID_INPUT",
			);
		}
	}

	edit([
		"resource",
		"qa",
		"primary-db",
		"changed.internal.invalid",
		"3307",
		"VERIFY_IDENTITY",
		"orders",
	]);
	edit(["revoke", "dev", "primary-db", "database.schema"]);
	assert.deepEqual(
		read().map((e) => e.target),
		["stage"],
	);
	assert.deepEqual(
		(await call("environment_list")).result.environments,
		read(),
	);
	invoke(["project", "delete", "--project", projectId], 0, true);
	assert.equal(
		invoke(["environment", "list", ...scope], 3).errorCode,
		"PROJECT_NOT_TRUSTED",
	);
	assert.equal(
		(await call("environment_list")).errorCode,
		"PROJECT_NOT_TRUSTED",
	);
	console.log(
		"T09: terminal approval, visible aliases, scoped grants, endpoint change and live project deletion passed.",
	);
} finally {
	processMcp.stdin.end();
	await closed;
}
