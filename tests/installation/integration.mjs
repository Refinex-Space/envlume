// Drive both native hosts against the disposable project and verified CLI from T14.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export function integrationReview({
  cli,
  desktop,
  tty,
  workspace,
  binding,
  tmp,
  env,
}) {
  const bins = join(tmp, "client discovery");
  mkdirSync(bins);
  const win = process.platform === "win32";
  const clean = Object.fromEntries(
    Object.entries(env).filter(([k]) => k.toUpperCase() !== "PATH"),
  );
  const receipt = {
    platform: process.platform,
    arch: process.arch,
    clients: [],
  };
  const secret = "T15_PRIVATE_FIXTURE_DO_NOT_PRINT";
  function invoke(
    program,
    args,
    { code = 0, approve = false, absent = false } = {},
  ) {
    const p = spawnSync(
      approve ? tty : program,
      [...(approve ? ["approve", program] : []), ...args, "--json"],
      {
        cwd: workspace,
        env: { ...clean, PATH: absent ? "" : bins },
        encoding: "utf8",
        timeout: 90000,
      },
    );
    assert.ok(
      !p.stdout?.includes(secret) && !p.stderr?.includes(secret),
      "third-party field escaped into output",
    );
    if (p.status !== code && approve) {
      const name = args[2];
      const rel = {
        codex: ".codex/config.toml",
        cursor: ".cursor/mcp.json",
        "claude-code": ".mcp.json",
      }[name];
      if (rel) {
        const file = join(workspace, rel);
        console.error(
          "T15 confirmation state",
          JSON.stringify({
            client: name,
            operation: args[1],
            entryVisible:
              existsSync(file) && readFileSync(file, "utf8").includes(binding),
          }),
        );
      }
    }
    assert.equal(
      p.status,
      code,
      JSON.stringify({
        operation: args.slice(0, 3),
        approve,
        processError: p.error?.code,
        diagnostic: (p.stderr ?? "").slice(-1300),
      }),
    );
    if (!approve) return JSON.parse(p.stdout);
  }
  for (const [client, exe, dir, file] of [
    ["codex", "codex", ".codex", "config.toml"],
    ["cursor", "cursor", ".cursor", "mcp.json"],
    ["claude-code", "claude", "", ".mcp.json"],
  ]) {
    // Detection only. Starting this sentinel executable would fail verification.
    writeFileSync(join(bins, exe + (win ? ".cmd" : "")), "exit 41\n", {
      mode: 0o700,
    });
    mkdirSync(join(workspace, dir), { recursive: true });
    const config = join(workspace, dir, file);
    const original =
      client === "codex"
        ? '# keep me\n[mcp_servers.other]\ncommand="' + secret + '"\n'
        : '{ "custom": [1,2], "mcpServers": {"other":{"value":"' +
          secret +
          '"}} }\r\n';
    writeFileSync(config, original);
    const options = (op) => [
      op,
      client,
      ...(op === "install" && client === "claude-code"
        ? ["--shared-project"]
        : []),
      "--binding",
      binding,
    ];
    const call = (op, opts) =>
      invoke(cli, ["integration", ...options(op)], opts);
    assert.equal(
      call("detect", { absent: true }).result.preview.detected,
      false,
    );
    assert.equal(
      call("install", { absent: true, code: 4 }).errorCode,
      "CLIENT_UNAVAILABLE",
    );
    assert.equal(call("preview").result.preview.change, "add");
    assert.equal(call("install", { code: 3 }).errorCode, "CANCELLED");
    assert.ok(
      readFileSync(config, "utf8") === original,
      "fixture original differs",
    );
    console.log("T15 native confirmation:", client);
    call("install", { approve: true });
    const verified = invoke(desktop, ["--integration", ...options("verify")]);
    assert.equal(verified.result.state, "CONFIGURED");
    assert.equal(verified.result.gatewayVerified, true);
    assert.equal(verified.result.clientVerified, false);
    assert.equal(verified.result.preview.launch.command, cli);
    const removePreview = invoke(cli, [
      "integration",
      "preview",
      client,
      "--uninstall",
      "--binding",
      binding,
    ]);
    assert.equal(removePreview.result.preview.change, "remove");
    const current = readFileSync(config, "utf8");
    assert.ok(current.includes(secret) && current.includes(binding));
    call("install", { approve: true });
    assert.ok(
      readFileSync(config, "utf8") === current,
      "registered fixture changed on repeated install",
    );
    invoke(desktop, ["--integration", ...options("uninstall")], {
      approve: true,
      absent: true,
    });
    assert.ok(
      readFileSync(config, "utf8") === original,
      "fixture original differs",
    );
    receipt.clients.push({
      client,
      scope: verified.result.preview.scope,
      key: verified.result.preview.key,
      state: verified.result.state,
      gatewayVerified: verified.result.gatewayVerified,
      clientVerified: verified.result.clientVerified,
      exactRestore: true,
    });
  }
  assert.equal(
    invoke(cli, ["integration", "verify", "unknown", "--binding", binding], {
      code: 2,
    }).errorCode,
    "INVALID_INPUT",
  );
  if (process.env.T15_TEST_SNAPSHOT)
    writeFileSync(
      process.env.T15_TEST_SNAPSHOT,
      JSON.stringify(receipt, null, 2) + "\n",
      { flag: "wx" },
    );
  console.log("T15 evidence " + JSON.stringify(receipt));
  console.log(
    "T15 native: three project layouts, real bound project_context, CLI/desktop, exact uninstall, cancellation and redacted JSON passed.",
  );
}
