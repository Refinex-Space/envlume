"""Exercise a real Codex app-server with a disposable, explicitly trusted project."""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import platform
from queue import Queue
import re
import sys
import subprocess
import tempfile
from threading import Thread

MARKER = "T17_SYNTHETIC_PRIVATE_SETTING"


@contextmanager
def session(exe, root, env, log):
    # Only a temporary user config and a fixture directory are passed by the caller.
    with log.open("wb") as stderr:
        p = subprocess.Popen(
            [exe, "app-server", "--listen", "stdio://"],
            cwd=root,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=stderr,
            text=True,
            encoding="utf-8",
        )
        messages = Queue()

        def reader():
            try:
                for line in p.stdout:
                    messages.put(json.loads(line))
            finally:
                messages.put(None)

        t = Thread(target=reader, daemon=True)
        t.start()
        n = 0

        def call(method, params):
            nonlocal n
            n += 1
            p.stdin.write(
                json.dumps({"method": method, "id": n, "params": params}) + "\n"
            )
            p.stdin.flush()
            while True:
                msg = messages.get(timeout=40)
                assert msg is not None
                if msg.get("id") == n:
                    return msg

        try:
            assert "result" in call(
                "initialize",
                {
                    "clientInfo": {"name": "envlume_t17", "version": "1"},
                    "capabilities": {"experimentalApi": True},
                },
            )
            yield call
        finally:
            p.stdin.close()
            try:
                p.wait(timeout=8)
            except subprocess.TimeoutExpired:
                p.kill()
                p.wait()
            t.join(timeout=2)
            p.stdout.close()


def review(cli, tty, codex, snapshot=None):
    with tempfile.TemporaryDirectory(prefix="envlume-t17-") as tmp:
        base = Path(tmp).resolve()
        root = base / "project with spaces"
        root.mkdir()
        profile = base / "Codex profile"
        profile.mkdir()
        env = {
            k: v
            for k, v in os.environ.items()
            if k.upper()
            in {
                "HOME",
                "USERPROFILE",
                "LOCALAPPDATA",
                "APPDATA",
                "SYSTEMROOT",
                "WINDIR",
                "PROGRAMFILES",
                "PROGRAMFILES(X86)",
                "PROGRAMDATA",
                "TEMP",
                "TMP",
                "TMPDIR",
                "COMSPEC",
                "SYSTEMDRIVE",
                "HOMEDRIVE",
                "HOMEPATH",
                "OS",
                "USERNAME",
                "USERDOMAIN",
                "NUMBER_OF_PROCESSORS",
                "PROCESSOR_ARCHITECTURE",
                "PATH",
            }
        }
        env.update(
            CODEX_HOME=str(profile),
            RUST_LOG="rmcp::service=info",
            HTTPS_PROXY="http://127.0.0.1:1",
            HTTP_PROXY="http://127.0.0.1:1",
        )
        env["PATH"] = str(codex.parent) + os.pathsep + env.get("PATH", "")

        def run(cmd, code=0):
            p = subprocess.run(
                cmd,
                cwd=root,
                env=env,
                stdin=subprocess.DEVNULL,
                capture_output=True,
                timeout=65,
            )
            assert MARKER.encode() not in p.stdout + p.stderr
            assert p.returncode == code, (
                cmd[1:3],
                p.returncode,
                re.findall(rb'"errorCode":"([A-Z_]+)"', p.stdout + p.stderr),
            )
            return p.stdout

        run(["git", "init", "--quiet"])
        run([tty, "approve", cli, "project", "add", root, "--json"])
        binding = json.loads(
            run([cli, "project", "inspect", "--path", root, "--json"])
        )["binding"]["bindingId"]
        suffix = ["--binding", binding, "--json"]

        def integration(op, approve=False, code=0):
            out = run(
                ([tty, "approve"] if approve else [])
                + [cli, "integration", op, "codex", *suffix],
                code,
            )
            return None if approve else json.loads(out)

        version = run([codex, "--version"]).decode().strip()
        expected = json.loads(run([cli, "project", "context", *suffix]))
        project_cfg = root / ".codex" / "config.toml"
        project_cfg.parent.mkdir()
        marker = base / "third-party-started"
        sentinel = base / "third_party.py"
        sentinel.write_text(
            "from pathlib import Path\nimport sys\nPath(sys.argv[1]).touch()\n",
            encoding="utf-8",
        )
        original = (
            '# preserve this comment\nmodel_reasoning_effort = "low"\n[mcp_servers.other]\ncommand = '
            + json.dumps(sys.executable)
            + "\nargs = "
            + json.dumps(
                [str(sentinel), str(marker), r"C:\Program Files\Other\third.exe"]
            )
            + '\n[mcp_servers.other.env]\nPRIVATE="'
            + MARKER
            + '"\n'
        ).encode()
        project_cfg.write_bytes(original)
        trust_file = profile / "config.toml"

        def trust(level):
            trust_file.write_text(
                "[projects."
                + json.dumps(str(root))
                + ']\ntrust_level="'
                + level
                + '"\n',
                encoding="utf-8",
            )
            return trust_file.read_bytes()

        # The exact target would be user scope when CODEX_HOME points at .codex.
        env["CODEX_HOME"] = str(project_cfg.parent)
        assert integration("preview", code=3)["errorCode"] == "SCOPE_CONFLICT"
        assert project_cfg.read_bytes() == original
        env["CODEX_HOME"] = str(profile)
        untrusted = trust("untrusted")
        assert (
            integration("preview")["result"]["preview"]["location"].replace("\\", "/")
            == ".codex/config.toml"
        )
        assert integration("install", code=3)["errorCode"] == "CANCELLED"
        assert project_cfg.read_bytes() == original
        integration("install", True, 1)
        assert (
            project_cfg.read_bytes() == original
            and trust_file.read_bytes() == untrusted
        )
        trusted = trust("trusted")
        integration("install", True)
        first = project_cfg.read_bytes()
        integration("install", True)
        assert project_cfg.read_bytes() == first and trust_file.read_bytes() == trusted
        verified = integration("verify")["result"]
        assert (
            verified["gatewayVerified"]
            and verified["clientConfigVerified"]
            and not verified["clientVerified"]
        )
        assert verified["state"] == "CONFIGURED"
        assert verified["preview"]["clientVersion"] == version.removeprefix(
            "codex-cli "
        )
        # A changed trust choice must be observed on the next verification.
        trust("untrusted")
        with trust_file.open("a", encoding="utf-8") as f:
            f.write(
                "\n[mcp_servers.envlume]\ncommand = "
                + json.dumps(str(cli))
                + "\nargs = "
                + json.dumps(["mcp", "--binding", binding])
                + "\n"
            )
        fallback = trust_file.read_bytes()
        assert (
            json.loads(run([codex, "mcp", "get", "envlume", "--json"]))["transport"][
                "args"
            ][-1]
            == binding
        )
        assert integration("verify", code=3)["errorCode"] == "CODEX_PROJECT_NOT_LOADED"
        assert project_cfg.read_bytes() == first and trust_file.read_bytes() == fallback
        trust("trusted")
        assert integration("verify")["result"]["clientConfigVerified"]
        integration("uninstall", True)
        assert (
            project_cfg.read_bytes() == original and trust_file.read_bytes() == trusted
        )
        run([codex, "mcp", "get", "envlume", "--json"], 1)
        integration("install", True)
        assert not marker.exists(), "Configuration verification started another server"
        debug = base / "client.log"
        with session(codex, root, env, debug) as rpc:
            thread = rpc(
                "thread/start",
                {
                    "cwd": str(root),
                    "ephemeral": True,
                    "sandbox": "read-only",
                    "approvalPolicy": "never",
                },
            )["result"]["thread"]["id"]
            target = {
                "threadId": thread,
                "serverName": "envlume",
                "detail": "toolsAndAuthOnly",
            }
            [server] = rpc("mcpServerStatus/list", target)["result"]["data"]
            assert (
                server["runtimeStatus"] == "connected"
                and "project_context" in server["tools"]
            )
            params = {
                "threadId": thread,
                "server": "envlume",
                "tool": "project_context",
                "arguments": {},
            }
            result = rpc("mcpServer/tool/call", params)["result"]
            report = result["structuredContent"]
            assert (
                not result["isError"]
                and report["ok"]
                and report["result"] == expected["result"]
                and report["bindingAlias"] == expected["bindingAlias"]
            )
            record = json.loads(
                run([cli, "report", "show", "--request", report["requestId"], *suffix])
            )
            assert (
                record["actor"] == "LOCAL_STDIO"
                and record["decision"] == "SUCCESS"
                and record["bindingId"] == binding
            )
            assert (
                json.loads(run([cli, "project", "revoke", *suffix]))["state"]
                == "REVOKED"
            )
            denied = rpc("mcpServer/tool/call", params)
            assert "BINDING_REVOKED" in json.dumps(
                denied
            ) and '"ok": true' not in json.dumps(denied)
        handshake = [
            line
            for line in debug.read_text(encoding="utf-8", errors="replace").splitlines()
            if "Service initialized as client" in line and 'name: "envlume"' in line
        ]
        versions = {
            m.group(1)
            for line in handshake
            if (
                m := re.search(
                    r'protocol_version: ProtocolVersion\("(\d{4}-\d{2}-\d{2})"\)', line
                )
            )
        }
        assert len(versions) == 1
        receipt = {
            "client": version,
            "os": platform.platform(),
            "scope": "trusted project",
            "protocol": versions.pop(),
            "tool": "project_context",
            "requestId": report["requestId"],
            "actor": record["actor"],
            "configRestored": True,
            "untrustedRejected": True,
            "liveRevocation": "BINDING_REVOKED",
            "modelTurns": 0,
            "userFallbackRejected": True,
            "configCheckStartedThirdParty": False,
        }
        if snapshot:
            with Path(snapshot).open("x", encoding="utf-8") as f:
                json.dump(receipt, f, indent=2)
                f.write("\n")
        print("T17 real-client evidence", json.dumps(receipt))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    for name in ["cli", "tty", "codex"]:
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--snapshot")
    args = parser.parse_args()
    review(args.cli.resolve(), args.tty.resolve(), args.codex.resolve(), args.snapshot)
