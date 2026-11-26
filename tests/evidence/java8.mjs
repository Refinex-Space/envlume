import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
	cpSync,
	mkdtempSync,
	mkdirSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { release, tmpdir } from "node:os";

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
const review = process.argv.includes("--review");
const helperArg = process.argv.slice(2).find((arg) => arg !== "--review");
const helper = helperArg ? resolve(helperArg) : fixtureBinary();
const cli = process.env.R1_NATIVE_CLI
	? resolve(process.env.R1_NATIVE_CLI)
	: helper;
const java = process.env.T08_JAVA8_HOME;
assert.ok(java, "Set T08_JAVA8_HOME to a complete Java 8 JDK.");
for (const tool of ["java", "javac"]) {
	const exe = join(
		java,
		"bin",
		process.platform === "win32" ? tool + ".exe" : tool,
	);
	const probe = spawnSync(exe, ["-version"], {
		encoding: "utf8",
		timeout: 60000,
	});
	assert.equal(
		probe.status,
		0,
		"The supplied JDK must have working java and javac.",
	);
	assert.match(probe.stdout + probe.stderr, /1\.8\.0_\d+/);
}
// A named, fresh local catalogue may be retained for the Phase 2 handoff.
const handoff = review && process.env.R1_HANDOFF_DIRECTORY;
if (handoff) {
	assert.equal(cli, helper, "Handoff uses the isolated fixture catalogue");
	mkdirSync(handoff, { mode: 0o700 });
}
const base = handoff || mkdtempSync(join(tmpdir(), "envlume-T08-"));
const cwd = join(base, "Java 8 Spring Boot");
const env = { ...process.env, T05_FIXTURE_STORE: join(base, "state") };
function invoke(args, confirm = false, origin = cwd) {
	const r = spawnSync(
		confirm ? helper : cli,
		[...(confirm ? ["approve", cli] : []), ...args, "--json"],
		{ cwd: origin, env, encoding: "utf8", timeout: 210000 },
	);
	assert.equal(r.status, 0, r.stderr + r.stdout);
	if (!confirm) return JSON.parse(r.stdout);
}
function probe(command, args) {
	const r = spawnSync(command, args, { encoding: "utf8", env, timeout: 60000 });
	assert.equal(r.status, 0, r.error?.message || r.stderr);
	return r.stdout + r.stderr;
}
function hostSnapshot() {
	// Also read the persistent host settings; a child cannot change its parent's environment.
	const persistent =
		process.platform === "win32"
			? probe("powershell.exe", [
					"-NoProfile",
					"-NonInteractive",
					"-Command",
					"foreach ($s in 'User','Machine') { foreach ($n in 'Path','JAVA_HOME') { [Console]::WriteLine([Environment]::GetEnvironmentVariable($n,$s)) } }",
				])
			: ["PATH", "JAVA_HOME"].map((n) => {
					const r = spawnSync("/bin/launchctl", ["getenv", n], {
						encoding: "utf8",
					});
					assert.ok(
						r.status === 0 || (r.status === 1 && !r.stderr),
						"launchctl getenv failed",
					);
					return [r.status, r.stdout];
				});
	return {
		path: process.env.PATH,
		javaHome: process.env.JAVA_HOME,
		persistent,
		java: probe("java", ["-version"]),
	};
}
function selectSibling(major) {
	const home = process.env["R1_JAVA" + major + "_HOME"];
	assert.ok(home, "R1 requires a real JDK " + major);
	const origin = join(base, "jdk-" + major);
	mkdirSync(origin);
	const bin = (name) =>
		join(home, "bin", process.platform === "win32" ? name + ".exe" : name);
	const version = probe(bin("java"), ["-version"]).match(
		/version "([^"]+)"/,
	)?.[1];
	assert.ok(version?.startsWith(major + "."));
	assert.ok(probe(bin("javac"), ["-version"]).includes(version));
	invoke(["project", "add", origin], true, origin);
	const binding = invoke(
		["project", "inspect", "--path", origin],
		false,
		origin,
	).binding.bindingId;
	invoke(
		[
			"toolchain",
			"configure",
			"--binding",
			binding,
			"--profile",
			"default",
			"--tool",
			"jdk",
			String(major),
			"--path",
			home,
		],
		true,
		origin,
	);
	return { major, origin, binding, version };
}
function selected(sibling) {
	const d = invoke(
		["doctor", "--binding", sibling.binding],
		false,
		sibling.origin,
	);
	const tool = d.result.tools.find((t) => t.tool === "jdk");
	assert.equal(tool.repair, null, JSON.stringify(tool));
	assert.equal(tool.version, sibling.version);
}
try {
	const host = review ? hostSnapshot() : null;
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
	const siblings = review ? [selectSibling(17), selectSibling(21)] : [];
	siblings.forEach(selected);
	if (review) {
		const tools = invoke(["doctor", ...scope]).result.tools;
		assert.match(tools.find((t) => t.tool === "jdk").version, /^1\.8\./);
		assert.match(tools.find((t) => t.tool === "maven").version, /^3\.9\./);
	}
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
	if (review) {
		invoke(["project", "revoke", ...scope]);
		siblings.forEach(selected);
		assert.deepEqual(hostSnapshot(), host);
		console.log(
			JSON.stringify({
				platform: process.platform,
				osRelease: release(),
				arch: process.arch,
				hostJava: host.java.match(/version "([^"]+)"/)?.[1],
				environmentUnchanged: true,
				siblings: siblings.map(({ major, binding, version }) => ({
					major,
					binding,
					version,
				})),
			}),
		);
	}
} finally {
	if (!handoff) rmSync(base, { recursive: true, force: true });
}
