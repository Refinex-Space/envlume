"""Real Claude Code client acceptance. Only the model conversation is a loopback fixture."""

import argparse
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tempfile

from claude_model import model_fixture, TOOL

CANARY = "T16_SYNTHETIC_THIRD_PARTY"
ACCOUNT = "T16_SYNTHETIC_PROFILE_ONLY"


def check(cli, tty, claude, workspace, binding, home_env, snapshot=None):
    # Use the same isolated native Envlume profile as the packaging test. Windows is a disposable CI user.
    with tempfile.TemporaryDirectory(prefix="envlume-t16-") as tmp, model_fixture() as (
        url,
        observed,
    ):
        base = Path(tmp).resolve()
        profile = base / "Claude profile 私有"
        profile.mkdir(mode=0o700)
        sibling = base / "other project"
        sibling.mkdir()
        env = {
            k: v
            for k, v in home_env.items()
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
            }
        }
        if os.name == "nt":
            discovery = claude.parent
        else:
            discovery = base / "bin"
            discovery.mkdir()
            (discovery / "claude").symlink_to(claude)
        env.update(
            CLAUDE_CONFIG_DIR=str(profile),
            DISABLE_AUTOUPDATER="1",
            CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1",
            CLAUDE_CODE_DISABLE_CLAUDE_AI_MCP="1",
            ANTHROPIC_BASE_URL=url,
            ANTHROPIC_API_KEY="t16-loopback-only",
            ENABLE_TOOL_SEARCH="false",
            HTTPS_PROXY="http://127.0.0.1:1",
            HTTP_PROXY="http://127.0.0.1:1",
            NO_PROXY="localhost,127.0.0.1",
            PATH=os.pathsep.join(
                [str(discovery), home_env.get("PATH", home_env.get("Path", ""))]
            ),
        )

        def run(cmd, cwd=workspace, code=0):
            if platform.system() == "Darwin" and cmd[0] == claude and "-p" in cmd:
                port = int(url.rsplit(":", 1)[1])
                policy = f'(version 1)(allow default)(deny network-outbound)(allow network-outbound (remote tcp "localhost:{port}"))'
                cmd = ["/usr/bin/sandbox-exec", "-p", policy, *cmd]
            p = subprocess.run(
                list(map(str, cmd)),
                cwd=cwd,
                env=env,
                stdin=subprocess.DEVNULL,
                capture_output=True,
                timeout=65,
            )
            assert all(v.encode() not in p.stdout + p.stderr for v in (CANARY, ACCOUNT))
            if p.returncode != code:
                diagnostics = re.findall(
                    rb'"errorCode":"([A-Z_]+)"|Confirmed CLI exited (.{1,60})',
                    p.stderr + p.stdout,
                )
                raise AssertionError(
                    f"{Path(cmd[0]).name} {cmd[1:3]} exited {p.returncode}, expected {code}; diagnostics {diagnostics!r}"
                )
            return p.stdout.decode("utf-8")

        def app(args, approve=False, code=0):
            out = run(
                ([tty, "approve"] if approve else [])
                + [cli, *args, "--binding", binding, "--json"],
                code=code,
            )
            assert CANARY not in out and ACCOUNT not in out
            return None if approve else json.loads(out)

        def integration(op, approve=False, code=0):
            return app(["integration", op, "claude-code"], approve, code)

        version = run([claude, "--version"]).strip()
        # Ask Claude itself to select the project key. Pre-existing unrelated values must survive.
        third = {"type": "http", "url": url + "/third", "headers": {"X-Test": CANARY}}
        run([claude, "mcp", "add-json", "--scope", "local", "third", json.dumps(third)])
        config = profile / ".claude.json"
        state = json.loads(config.read_text(encoding="utf-8"))
        [project_key] = state["projects"]
        # A first-ever health check initializes this field and reformats the file.
        # Use an already-initialized synthetic profile for the exact-byte rollback case.
        state.setdefault("userID", "0" * 64)
        state["fixtureUnknown"] = {"private": ACCOUNT}
        state["projects"][project_key]["fixturePreference"] = True
        config.write_text(json.dumps(state, ensure_ascii=False), encoding="utf-8")

        def state_of_project():
            data = json.loads(config.read_text(encoding="utf-8"))
            assert data["fixtureUnknown"] == {"private": ACCOUNT}
            assert data["projects"][project_key]["fixturePreference"] is True
            servers = data["projects"][project_key]["mcpServers"]
            assert servers["third"] == third
            return servers

        def call_real():
            observed["advertised"] = False
            observed["toolResults"].clear()
            log = base / "client-debug.log"
            events = [
                json.loads(l)
                for l in run(
                    [
                        claude,
                        "-p",
                        "Call the project context tool once.",
                        "--tools",
                        "",
                        "--allowedTools",
                        TOOL,
                        "--permission-mode",
                        "dontAsk",
                        "--model",
                        "claude-sonnet-4-6",
                        "--no-session-persistence",
                        "--verbose",
                        "--output-format",
                        "stream-json",
                        "--debug-file",
                        log,
                    ]
                ).splitlines()
            ]
            assert any(
                e.get("type") == "system"
                and any(
                    s.get("name") == "envlume" and s.get("status") == "connected"
                    for s in e.get("mcp_servers", [])
                )
                for e in events
            )
            assert observed["advertised"] and len(observed["toolResults"]) == 1
            result = observed["toolResults"][0]
            assert not result.get("is_error")
            content = result["content"]
            if isinstance(content, list):
                content = next(c["text"] for c in content if c.get("type") == "text")
            report, _ = json.JSONDecoder().raw_decode(content)
            assert report["ok"] and report["operation"] == "project_context"
            assert (
                report["bindingAlias"] == expected["bindingAlias"]
                and report["result"] == expected["result"]
            )
            record = app(["report", "show", "--request", report["requestId"]])
            assert (
                record["bindingId"] == binding
                and record["actor"] == "LOCAL_STDIO"
                and record["decision"] == "SUCCESS"
            )
            prefix = 'MCP server "envlume": Connection established with capabilities: '
            [connection] = [
                json.loads(line.split(prefix, 1)[1])
                for line in log.read_text(encoding="utf-8").splitlines()
                if prefix in line
            ]
            assert (
                connection["serverVersion"]["name"] == "envlume"
                and connection["hasTools"]
            )
            protocol = connection["negotiatedProtocolVersion"]
            assert re.fullmatch(r"\d{4}-\d{2}-\d{2}", protocol)
            return {
                "requestId": report["requestId"],
                "bindingAlias": report["bindingAlias"],
                "protocol": protocol,
                "tool": TOOL,
            }

        backups = cli.parent.parent / "backups"
        previous_backups = set(backups.glob("integration-*.bak"))
        expected = app(["project", "context"])
        assert not (workspace / ".mcp.json").exists()
        preview = integration("preview")["result"]["preview"]
        assert preview["scope"] == "local" and preview["change"] == "add"
        assert "envlume" not in state_of_project()
        assert integration("install", code=3)["errorCode"] == "CANCELLED"
        state = json.loads(config.read_text(encoding="utf-8"))
        project = state["projects"][project_key]
        project["disabledMcpServers"] = ["envlume"]
        config.write_text(json.dumps(state), encoding="utf-8")
        disabled_before = config.read_bytes()
        integration("install", True, code=1)
        assert (
            config.read_bytes() == disabled_before
            and "envlume" not in state_of_project()
        )
        assert integration("detect")["result"]["preview"]["change"] == "add"
        del project["disabledMcpServers"]
        config.write_text(json.dumps(state), encoding="utf-8")
        integration("install", True)
        installed = integration("verify")["result"]
        assert (
            installed["state"] == "CONNECTED"
            and installed["clientVerified"]
            and installed["gatewayVerified"]
        )
        assert Path(state_of_project()["envlume"]["command"]) == cli
        integration("install", True)
        attestation = call_real()
        negative = subprocess.run(
            [claude, "mcp", "get", "envlume"],
            cwd=sibling,
            env=env,
            capture_output=True,
            timeout=25,
        )
        assert negative.returncode != 0 and b"Connected" not in negative.stdout
        data = json.loads(config.read_text(encoding="utf-8"))
        data["projects"].setdefault(str(sibling), {})["fixtureExternalEdit"] = ACCOUNT
        config.write_text(json.dumps(data), encoding="utf-8")
        integration("uninstall", True)
        assert (
            json.loads(config.read_text(encoding="utf-8"))["projects"][str(sibling)][
                "fixtureExternalEdit"
            ]
            == ACCOUNT
        )
        assert (
            "envlume" not in state_of_project()
            and not (workspace / ".mcp.json").exists()
        )
        removed = subprocess.run(
            [claude, "mcp", "get", "envlume"],
            cwd=workspace,
            env=env,
            capture_output=True,
            timeout=25,
        )
        assert removed.returncode != 0
        backups_created = set(backups.glob("integration-*.bak")) - previous_backups
        assert len(backups_created) == 4
        for backup in backups_created:
            raw = backup.read_bytes()
            assert all(
                s.encode() not in raw for s in (CANARY, ACCOUNT, "t16-loopback-only")
            )
            assert set(json.loads(raw)) in ({"offset", "length"}, {"command", "args"})
        # A Git subdirectory may be a trusted Envlume workspace while Claude's
        # local MCP scope belongs to the enclosing repository.
        repo = base / "repository"
        nested = repo / "nested"
        nested.mkdir(parents=True)
        run(["git", "init", "--quiet", repo], cwd=base)
        run([tty, "approve", cli, "project", "add", nested, "--json"], cwd=nested)
        inspection = json.loads(
            run([cli, "project", "inspect", "--path", nested, "--json"], cwd=nested)
        )
        nested_id = inspection["binding"]["bindingId"]
        denied = json.loads(
            run(
                [
                    cli,
                    "integration",
                    "preview",
                    "claude-code",
                    "--binding",
                    nested_id,
                    "--json",
                ],
                cwd=nested,
                code=3,
            )
        )
        assert denied["errorCode"] == "PROJECT_SCOPE_MISMATCH"

        receipt = {
            "gitSubdirectoryScopeMismatch": True,
            "rollbackVerified": True,
            "privateBackupsChecked": len(backups_created),
            "client": version,
            "os": platform.platform(),
            "arch": platform.machine(),
            "scope": "local",
            "clientReload": "new process",
            "modelTransport": "loopback scripted Messages",
            "projectContext": attestation,
            "siblingVisible": False,
            "thirdPartyPreserved": True,
        }
        if snapshot:
            with Path(snapshot).open("x") as f:
                json.dump(receipt, f, indent=2)
                f.write("\n")
        print("T16 real-client evidence", json.dumps(receipt))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    for key in ["cli", "tty", "claude", "workspace", "binding"]:
        parser.add_argument("--" + key, required=True)
    parser.add_argument("--snapshot")
    options = parser.parse_args()
    check(
        Path(options.cli).resolve(),
        Path(options.tty).resolve(),
        Path(options.claude).resolve(),
        Path(options.workspace).resolve(),
        options.binding,
        os.environ,
        options.snapshot,
    )
