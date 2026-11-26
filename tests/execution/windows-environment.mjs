import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

// Exercise the installed npm's Windows lifecycle shell under an empty-parent environment.
const temp = mkdtempSync(join(tmpdir(), "envlume-node-"));
try {
  const root = join(temp, "T07 空格 build");
  const home = join(temp, "home");
  mkdirSync(root);
  mkdirSync(home);
  writeFileSync(join(root, "package.json"), JSON.stringify({name: "fixture", scripts: {build: "node build.cjs"}}));
  writeFileSync(join(root, "build.cjs"), 'process.stdout.write("fixture-built")');
  const bin = dirname(process.execPath);
  const win = process.env.SystemRoot;
  const npm = join(bin, "node_modules", "npm", "bin", "npm-cli.js");
  const env = {
    SystemRoot: win, WINDIR: win, ComSpec: join(win, "System32", "cmd.exe"),
    PATHEXT: ".COM;.EXE;.BAT;.CMD", PATH: [bin, join(win, "System32"), join(win, "System32/WindowsPowerShell/v1.0")].join(";"),
    HOME: home, USERPROFILE: home, LOCALAPPDATA: home, APPDATA: home,
    TMP: temp, TEMP: temp, TMPDIR: temp, NODE: process.execPath, CI: "true",
  };
  const result = spawnSync(process.execPath, [npm, "run", "build"], {cwd: root, env, encoding: "utf8"});
  assert.equal(result.status, 0, result.stderr);
  assert.ok(result.stdout.includes("fixture-built"));
} finally {
  rmSync(temp, { recursive: true, force: true });
}
