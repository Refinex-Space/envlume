"""Phase 2 gate. All resources and credentials belong to the disposable MySQL fixture."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack, closing
import json
import os
from pathlib import Path
import platform
import secrets
import select
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time

from review import binary, mysql_fixture, recv, send, wait_for


class Scan:
    def __init__(self, values):
        self.needles = [s.encode(enc) for s in values for enc in ("utf-8", "utf-16le")]
        self.sizes = {}

    def check(self, channel, data):
        if any(needle in data for needle in self.needles):
            raise AssertionError(f"private fixture value on {channel}")
        self.sizes[channel] = self.sizes.get(channel, 0) + len(data)


class Client:
    def __init__(self, project, exe, tty, store, scan):
        self.binding = project["binding"]
        self.project = project["project"]
        self.root = Path(project["workspace"])
        self.scope = ["--binding", self.binding]
        self.exe, self.tty, self.scan = exe, tty, scan
        self.env = {**os.environ, "T05_FIXTURE_STORE": str(store)}

    def cli(self, args, *, approve=False, code=0, data=None, scope=None):
        cmd = ([self.tty, "approve"] if approve else []) + [
            self.exe,
            *args,
            *(self.scope if scope is None else scope),
            "--json",
        ]
        p = subprocess.run(
            cmd,
            cwd=self.root,
            env=self.env,
            input=data,
            capture_output=True,
            timeout=90,
        )
        self.scan.check("CLI stderr", p.stderr)
        self.scan.check("CLI stdout", p.stdout)
        assert p.returncode == code, f"{args[0]} {args[1]} exited {p.returncode}"
        if not approve:
            return json.loads(p.stdout)

    def prepare_node(self, output_marker):
        (self.root / "package.json").write_text(
            json.dumps({"name": "r2-fixture", "scripts": {"build": "node build.cjs"}})
        )
        (self.root / "package-lock.json").write_text("{}")
        (self.root / "build.cjs").write_text("""
const fs = require('node:fs');
const cp = require('node:child_process');
console.log(%s);
console.error(%s);
if (fs.existsSync('hold')) {
  fs.writeFileSync('parent.pid', String(process.pid));
  cp.spawn(process.execPath,['-e',"require('node:fs').writeFileSync('child.pid', String(process.pid)); setInterval(()=>{},1000)"], {stdio:'inherit'});
  setInterval(()=>{},1000);
}
""" % (json.dumps(output_marker), json.dumps(output_marker)))
        self.cli(
            [
                "toolchain",
                "configure",
                *self.scope,
                "--profile",
                "default",
                "--tool",
                "node",
                "*",
                "--path",
                shutil.which("node"),
            ],
            approve=True,
            scope=[],
        )
        self.cli(["task", "enable", "node-build"], approve=True)
        self.cli(["task", "run", "node-build"], approve=True)

    def record(self, report, actor):
        assert report["ok"] and report["requestId"] and report["evidenceId"]
        row = self.cli(["report", "show", "--evidence", report["evidenceId"]])
        assert row == self.cli(["report", "show", "--request", report["requestId"]])
        assert (row["requestId"], row["bindingId"], row["projectId"], row["actor"]) == (
            report["requestId"],
            self.binding,
            self.project,
            actor,
        )
        assert row["query"]["maskedColumns"] == 1 and row["query"]["rowCount"] == 1
        return {
            k: row[k]
            for k in (
                "requestId",
                "evidenceId",
                "bindingId",
                "projectId",
                "resourceId",
                "targetId",
                "actor",
            )
        }


class Peer:
    def __init__(self, client):
        self.client = client
        self.seq = 0
        self.p = subprocess.Popen(
            [client.exe, "mcp", *client.scope],
            cwd=client.root,
            env=client.env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def request(self, method, params):
        self.seq += 1
        send(
            self.p,
            {"jsonrpc": "2.0", "id": self.seq, "method": method, "params": params},
        )

    def response(self):
        if not select.select([self.p.stdout], [], [], 30)[0]:
            raise TimeoutError("MCP response timed out")
        wire = self.p.stdout.readline()
        self.client.scan.check("MCP stdout", wire)
        assert all(
            v not in wire
            for v in (b"127.0.0.1", b"BEGIN CERTIFICATE", os.fsencode(self.client.root))
        )
        response = json.loads(wire)
        assert response["jsonrpc"] == "2.0" and response["id"] == self.seq
        return response["result"]

    def call(self, name, args):
        self.request("tools/call", {"name": name, "arguments": args})
        response = self.response()
        report = response["structuredContent"]
        assert response["isError"] is (not report["ok"])
        return report

    def start(self, target):
        self.request(
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "clientInfo": {"name": "r2", "version": "1"},
                "capabilities": {},
            },
        )
        assert self.response()["protocolVersion"] == "2025-11-25"
        send(self.p, {"jsonrpc": "2.0", "method": "notifications/initialized"})
        ctx = self.call("project_context", {})
        local = self.client.cli(["project", "context"])
        assert (
            ctx["ok"]
            and ctx["result"] == local["result"]
            and ctx["result"]["targets"] == [target]
        )
        assert ctx["bindingAlias"] == ctx["result"]["workspace"]
        assert (
            self.client.cli(["report", "show", "--request", ctx["requestId"]])[
                "projectId"
            ]
            == self.client.project
        )
        return ctx["bindingAlias"]

    def close(self):
        if self.p.stdin is None:
            return
        self.p.stdin.close()
        self.p.stdin = None
        try:
            out, err = self.p.communicate(timeout=20)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.communicate()
            raise
        self.client.scan.check("MCP stderr", err)
        self.client.scan.check("MCP stdout", out)
        for line in out.splitlines():
            assert json.loads(line)["jsonrpc"] == "2.0"
        assert self.p.returncode == 0


def args(target, resource="primary-db"):
    return {"target": target, "resource": resource}


def query(target, resource="primary-db"):
    return {
        **args(target, resource),
        "sql": f"SELECT payload,name FROM {target}_marker WHERE id = 1",
    }


def active(sql):
    return int(
        sql(
            "SELECT COUNT(*) FROM performance_schema.threads WHERE PROCESSLIST_USER IN ('t11_dev','t11_test')"
        )
    )


def counters(sql):
    rows = sql(
        """SELECT USER,TOTAL_CONNECTIONS FROM performance_schema.users
        WHERE USER IN ('t11_dev','t11_test') ORDER BY USER;
        SELECT USER,SUM(COUNT_STAR) FROM performance_schema.events_statements_summary_by_account_by_event_name
        WHERE USER IN ('t11_dev','t11_test') GROUP BY USER ORDER BY USER"""
    ).splitlines()
    pairs = [row.split("\t") for row in rows]
    assert [u for u, _ in pairs] == ["t11_dev", "t11_test"] * 2
    counts = [int(n) for _, n in pairs]
    assert all(n > 0 for n in counts)
    return counts


def exercise(a, b, sql, lock, store, output_marker):
    receipt = {}
    with ExitStack() as stack:
        pa, pb = Peer(a), Peer(b)
        stack.callback(pa.close)
        stack.callback(pb.close)
        assert pa.start("dev") != pb.start("test")
        assert a.binding != b.binding and a.project != b.project
        assert active(sql) == 0
        # The two resources have identical aliases but different real rows and logins.
        for client, peer, target in [(a, pa, "dev"), (b, pb, "test")]:
            client.prepare_node(output_marker)
            d = peer.call("toolchain_doctor", {"profile": "default"})
            assert d["ok"] and d["result"] == client.cli(["doctor"])["result"]
            assert peer.call("connection_test", args(target))["ok"]
            schema = peer.call("database_schema", args(target))["result"]["tables"]
            assert [t["table"] for t in schema] == [f"{target}_marker"]
            assert [c["name"] for c in schema[0]["columns"]] == [
                "id",
                "name",
                "payload",
            ]
        assert (
            sql(
                """SELECT DISTINCT PROCESSLIST_USER,PROCESSLIST_DB FROM performance_schema.threads
            WHERE PROCESSLIST_USER IN ('t11_dev','t11_test') ORDER BY PROCESSLIST_USER"""
            ).splitlines()
            == ["t11_dev\torders_dev", "t11_test\torders_test"]
        )
        assert pb.call("database_query", query("test", "ledger"))["result"]["rows"] == [
            ["R2-TEST", "[REDACTED]"]
        ]
        before = counters(sql)
        denied = {}
        for label, peer, tool, inputs, expected in [
            (
                "A_B_only",
                pa,
                "database_query",
                query("test", "ledger"),
                "TARGET_NOT_ALLOWED",
            ),
            ("A_TEST", pa, "database_query", query("test"), "POLICY_DENIED"),
            ("B_DEV", pb, "database_query", query("dev"), "POLICY_DENIED"),
            (
                "implicit_target",
                pa,
                "database_query",
                {"resource": "primary-db", "sql": "SELECT id FROM dev_marker"},
                "INVALID_INPUT",
            ),
            ("PROD_connect", pa, "connection_test", args("prod"), "TARGET_NOT_ALLOWED"),
            ("PROD_schema", pb, "database_schema", args("prod"), "TARGET_NOT_ALLOWED"),
            ("PROD_query", pa, "database_query", query("prod"), "TARGET_NOT_ALLOWED"),
            (
                "PROD_write",
                pb,
                "database_query",
                {**args("prod"), "sql": "UPDATE test_marker SET id=0 WHERE id=0"},
                "TARGET_NOT_ALLOWED",
            ),
            (
                "caller_binding",
                pa,
                "connection_test",
                {**args("dev"), "bindingId": b.binding},
                "INVALID_INPUT",
            ),
            (
                "cross_schema",
                pa,
                "database_query",
                {**args("dev"), "sql": "SELECT id FROM orders_test.test_marker"},
                "QUERY_NOT_ALLOWED",
            ),
            (
                "write",
                pa,
                "database_query",
                {**args("dev"), "sql": "UPDATE dev_marker SET id=0 WHERE id=0"},
                "QUERY_NOT_ALLOWED",
            ),
        ]:
            report = peer.call(tool, inputs)
            assert (
                report.get("errorCode") == expected and "evidenceId" not in report
            ), label
            denied[label] = expected
        assert (
            a.cli(
                ["database", "query", "test", "primary-db"],
                code=3,
                data=b"SELECT id FROM test_marker",
            )["errorCode"]
            == "POLICY_DENIED"
        )
        assert (
            a.cli(["connection", "test", "test", "primary-db"], scope=b.scope, code=3)[
                "errorCode"
            ]
            == "PROJECT_NOT_TRUSTED"
        )
        assert (
            a.cli(
                ["environment", "grant", "prod", "primary-db", "database.read"], code=3
            )["errorCode"]
            == "TARGET_NOT_ALLOWED"
        )
        assert counters(sql) == before, "denied request reached MySQL"
        receipt["denied"] = denied
        receipt["denialCounterDeltas"] = dict.fromkeys(
            ["devConnections", "testConnections", "devStatements", "testStatements"], 0
        )

        # Hold both marker tables until all three independent callers are waiting inside mysqld.
        with ThreadPoolExecutor(max_workers=3) as pool:
            with lock(both=True):
                first = pool.submit(pa.call, "database_query", query("dev"))
                second = pool.submit(pb.call, "database_query", query("test"))
                third = pool.submit(
                    a.cli,
                    ["database", "query", "dev", "primary-db"],
                    data=query("dev")["sql"].encode(),
                )
                wait_for(
                    lambda: sql(
                        """SELECT COUNT(*) FROM performance_schema.threads
                    WHERE PROCESSLIST_USER IN ('t11_dev','t11_test') AND PROCESSLIST_STATE='Waiting for table metadata lock'"""
                    )
                    == "3",
                    "three queries did not overlap",
                    4,
                )
            reports = [f.result() for f in (first, second, third)]
        for r, marker in zip(reports, ["R2-DEV", "R2-TEST", "R2-DEV"]):
            assert r["result"]["rows"] == [[marker, "[REDACTED]"]]
        records = [
            client.record(report, actor)
            for client, report, actor in zip(
                [a, b, a], reports, ["LOCAL_STDIO", "LOCAL_STDIO", "CLI"]
            )
        ]
        for field in ("requestId", "evidenceId"):
            assert len({r[field] for r in records}) == 3
        assert (
            records[0]["resourceId"]
            == records[2]["resourceId"]
            != records[1]["resourceId"]
        )
        assert (
            a.cli(["report", "show", "--evidence", records[1]["evidenceId"]], code=3)[
                "errorCode"
            ]
            == "POLICY_DENIED"
        )
        receipt["concurrentRecords"] = records
        with lock():
            timeout = pb.call("database_query", query("test"))
            assert timeout["errorCode"] == "TIMEOUT"
        assert pb.call("database_query", query("test"))["ok"]
        receipt["timeoutMs"] = timeout["durationMs"]
        a.cli(
            ["environment", "revoke", "dev", "primary-db", "database.read"],
            approve=True,
        )
        before = counters(sql)
        assert pa.call("database_query", query("dev"))["errorCode"] == "POLICY_DENIED"
        assert counters(sql) == before
        assert (
            a.cli(["report", "show", "--evidence", records[0]["evidenceId"]])["actor"]
            == "LOCAL_STDIO"
        )
        # EOF during a real MCP task must reap its running child tree and release its slot.
        (a.root / "hold").touch()
        pa.request(
            "tools/call", {"name": "task_run", "arguments": {"taskId": "node-build"}}
        )
        pid_files = [a.root / "parent.pid", a.root / "child.pid"]
        wait_for(
            lambda: all(f.exists() and f.stat().st_size for f in pid_files),
            "MCP task descendants never started",
            20,
        )
        pids = [int(f.read_text()) for f in pid_files]
        markers = list(
            (store / "tmp").glob(f"task-slot-*-{a.binding}-node-build-*.run")
        )
        assert len(markers) == 1, "running task slot was not observed"
        pa.close()

        def gone():
            for pid in pids:
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    continue
                return False
            return True

        wait_for(gone, "MCP task descendants survived EOF", 12)
        wait_for(
            lambda: not markers[0].exists()
            and not markers[0].with_suffix(".stop").exists(),
            "task slot marker survived EOF",
        )
        wait_for(
            lambda: sql(
                "SELECT COUNT(*) FROM performance_schema.threads WHERE PROCESSLIST_USER='t11_dev'"
            )
            == "0",
            "A kept its pool after EOF",
        )
        (a.root / "hold").unlink()
        assert a.cli(["task", "run", "node-build"])["result"]["exitCode"] == 0
        assert pb.call("connection_test", args("test"))["ok"]
    wait_for(lambda: active(sql) == 0, "MySQL sessions survived client exit", 12)
    receipt["cleanup"] = {
        "mysqlSessions": 0,
        "taskPidsReaped": len(pids),
        "taskSlotReused": True,
    }
    return receipt


def review(snapshot, mysql_bin):
    if sys.platform != "darwin":
        raise SystemExit("Run this native Keychain/Docker fixture on macOS.")
    exe = binary("envlume", "mysql_review", "--example", "mysql_review")
    tty = binary("envlume", "workspace_trust", "--test", "workspace_trust")
    with tempfile.TemporaryDirectory(prefix="envlume-r2-") as tmp:
        base = Path(tmp)
        with mysql_fixture(base, mysql_bin) as (cfg, sql, lock):
            masked = secrets.token_hex(22)
            output_marker = secrets.token_hex(25)
            for target in ("dev", "test"):
                sql(
                    f"UPDATE orders_{target}.{target}_marker SET name='{masked}',payload='R2-{target.upper()}' WHERE id=1"
                )
            scan = Scan(
                [
                    masked,
                    output_marker,
                    *[v for login in cfg["accounts"].values() for v in login.values()],
                ]
            )
            cfg.update(base=tmp, phase="R2")
            with subprocess.Popen(
                [exe],
                cwd=base,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            ) as proc:
                try:
                    send(proc, cfg)
                    setup = recv(proc, 120)
                    store = Path(setup["store"])
                    a, b = [
                        Client(project, exe, tty, store, scan)
                        for project in setup["projects"]
                    ]
                    path = store / "data/envlume.db"
                    with closing(sqlite3.connect(path)) as pin:
                        pin.execute("BEGIN")
                        assert pin.execute(
                            "SELECT count(*) FROM projects"
                        ).fetchone() == (2,)
                        assert pin.execute(
                            "SELECT count(*) FROM environment_targets WHERE kind='PROD' AND risk_level='CRITICAL'"
                        ).fetchone() == (2,)
                        receipt = exercise(a, b, sql, lock, store, output_marker)
                        backup = store / "backups" / "r2-audit.db"
                        with closing(sqlite3.connect(path)) as src, closing(
                            sqlite3.connect(backup)
                        ) as dest:
                            src.backup(dest)
                            assert dest.execute("PRAGMA quick_check").fetchone() == (
                                "ok",
                            )
                            assert (
                                dest.execute(
                                    "SELECT count(*) FROM execution_records"
                                ).fetchone()[0]
                                > 0
                            )
                        for label, file in [
                            ("SQLite", path),
                            ("WAL", Path(str(path) + "-wal")),
                            ("online backup", backup),
                        ]:
                            data = file.read_bytes()
                            assert data
                            scan.check(label, data)
                        logs = list(store.joinpath("logs").rglob("*")) + list(
                            store.joinpath("tmp").rglob("*.log")
                        )
                        for file in logs:
                            if file.is_file():
                                scan.check("local logs", file.read_bytes())
                        scan.sizes.setdefault("local logs", 0)
                finally:
                    proc.stdin.close()
                    proc.stdin = None
                    out, err = proc.communicate(timeout=60)
                    scan.check("provisioner stderr", err)
                    scan.check("provisioner stdout", out)
                    assert (
                        proc.returncode == 0
                    ), "native fixture provisioning/cleanup failed"
            with closing(sqlite3.connect(path)) as db:
                assert db.execute(
                    "SELECT count(*) FROM resource_bindings WHERE credential_ref IS NOT NULL"
                ).fetchone() == (0,)
            receipt.update(
                mysql=sql("SELECT VERSION()"),
                platform=platform.mac_ver()[0],
                architecture=platform.machine(),
                scanBytes=scan.sizes,
                sqlAccountGrants="USAGE and table SELECT only; direct INSERT and cross-database SELECT denied (1142)",
                observedConcurrentWaiters=3,
            )
            print("R2 evidence " + json.dumps(receipt, sort_keys=True), flush=True)
            if snapshot:
                with snapshot.open("x") as file:
                    json.dump(receipt, file, indent=2)
                    file.write("\n")
            print(
                "R2: two projects, two MySQL readers, three simultaneous queries, evidence isolation, secret scan and EOF cleanup passed."
            )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument(
        "--mysql-bin-dir",
        type=Path,
        help="launch a private native MySQL 8.4 server instead of Docker",
    )
    opts = parser.parse_args()
    review(opts.snapshot, opts.mysql_bin_dir)
