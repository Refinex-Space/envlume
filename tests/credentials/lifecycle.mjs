// Native provider, with only test-generated logins submitted through a private PTY.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
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
const cmd = (op) => ["credential", op, "dev", "db", ...scope];
assert.equal(call(cmd("status")).state, "NOT_SET");
assert.equal(call(cmd("set"), 3).state, "ACCESS_DENIED");
try {
	call(cmd("set"), 0, true);
	assert.equal(call(cmd("status")).state, "AVAILABLE");
	call(cmd("replace"), 0, true);
	assert.equal(call(cmd("status")).state, "AVAILABLE");
} finally {
	call(cmd("clear"), 0, true);
}
assert.equal(call(cmd("status")).state, "NOT_SET");
call(["credential", "cleanup", ...scope], 0, true);
console.log(
	"T10 CLI: terminal-only submission, status, rotation and cleanup passed without exposing the test login.",
);
