import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
	cpSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

function fixtureBinary() {
	const build = spawnSync(
		"cargo",
		[
			"test",
			"--locked",
			"-p",
			"envlume",
			"--test",
			"workspace_trust",
			"--no-run",
			"--message-format=json",
		],
		{
			cwd: fileURLToPath(new URL("../../", import.meta.url)),
			encoding: "utf8",
			maxBuffer: 16 * 1024 * 1024,
		},
	);
	assert.equal(build.status, 0, build.stderr);
	const fixture = build.stdout
		.trim()
		.split("\n")
		.map((line) => JSON.parse(line))
		.find(
			(record) => record.executable && record.target.name === "workspace_trust",
		);
	assert.ok(fixture, "Cargo did not produce the native fixture binary");
	return fixture.executable;
}
const helper = process.argv[2] ? resolve(process.argv[2]) : fixtureBinary();
const java = process.env.T08_JAVA8_HOME;
assert.ok(java, "Set T08_JAVA8_HOME to a complete Java 8 JDK.");
const base = mkdtempSync(join(tmpdir(), "envlume-T08-"));
const cwd = join(base, "Java 8 Spring Boot");
const env = { ...process.env, T05_FIXTURE_STORE: join(base, "state") };
function invoke(args, confirm = false) {
	const r = spawnSync(
		helper,
		[...(confirm ? ["approve", helper] : []), ...args, "--json"],
		{ cwd, env, encoding: "utf8", timeout: 210000 },
	);
	assert.equal(r.status, 0, r.stderr + r.stdout);
	if (!confirm) return JSON.parse(r.stdout);
}
try {
	cpSync(fileURLToPath(new URL("spring-boot-java8", import.meta.url)), cwd, {
		recursive: true,
	});
	invoke(["project", "add", cwd], true);
	const id = invoke(["project", "inspect", "--path", cwd]).binding.bindingId;
	const scope = ["--binding", id];
	invoke(
		[
			"toolchain",
			"configure",
			...scope,
			"--profile",
			"default",
			"--tool",
			"jdk",
			"8",
			"--path",
			java,
			"--tool",
			"maven",
			"3.9",
		],
		true,
	);
	const run = ["task", "run", "maven-test", ...scope];
	invoke(["task", "enable", "maven-test", ...scope], true);
	invoke(run, true);
	for (const fails of [false, true]) {
		if (fails) writeFileSync(join(cwd, "fail-assertion"), "");
		const result = invoke(run);
		const record = invoke([
			"report",
			"show",
			"--evidence",
			result.evidenceId,
			...scope,
		]);
		const independent = readFileSync(
			join(
				cwd,
				"target",
				"surefire-reports",
				"TEST-example.ApplicationTest.xml",
			),
			"utf8",
		);
		const suite = independent.match(/<testsuite\b[^>]+>/)?.[0];
		assert.ok(suite);
		assert.match(suite, /tests="1"/);
		assert.match(suite, /errors="0"/);
		assert.ok(suite.includes('failures="' + Number(fails) + '"'));
		const version = independent.match(
			/<property name="java.version" value="([^"]+)"/,
		)?.[1];
		assert.match(version, /^1\.8\./);
		assert.equal(record.versions.jdk, version);
		assert.equal(record.requestId, result.requestId);
		assert.equal(record.evidenceId, result.evidenceId);
		assert.equal(record.durationMs, result.durationMs);
		assert.ok(result.durationMs > 0);
		assert.equal(record.exitCode, result.result.exitCode);
		assert.equal(record.exitCode === 0, !fails);
		assert.equal(record.decision, fails ? "FAILED" : "SUCCESS");
		const counts =
			record.stdoutSummary.assertions ?? record.stderrSummary.assertions;
		assert.deepEqual(counts, {
			tests: 1,
			failures: Number(fails),
			errors: 0,
			skipped: 0,
		});
		console.log(
			JSON.stringify({
				decision: record.decision,
				requestId: record.requestId,
				evidenceId: record.evidenceId,
				versions: record.versions,
				durationMs: record.durationMs,
				exitCode: record.exitCode,
				assertions: counts,
			}),
		);
	}
} finally {
	rmSync(base, { recursive: true, force: true });
}
