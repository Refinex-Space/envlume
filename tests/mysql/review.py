"""Local native T11 acceptance. Uses one private Docker container, two read-only accounts."""

import contextlib
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import uuid

REPO = Path(__file__).resolve().parents[2]
IMAGE = "mysql:8.4.11@sha256:0f51e63e537a1f176423fa03ef559aabe6fa4372a53160a44bc7e75da7d15364"


def run(args, *, data=None, cwd=None, env=None, timeout=90):
    proc = subprocess.run(
        args, input=data, capture_output=True, cwd=cwd, env=env, timeout=timeout
    )
    if proc.returncode:
        # Docker/MySQL can put a query or user name in stderr.
        raise RuntimeError(f"{Path(args[0]).name} exited {proc.returncode}")
    return proc.stdout


@contextlib.contextmanager
def docker_server(base):
    tls, admin = base / "tls", base / "admin"
    name = "envlume-t11-" + uuid.uuid4().hex
    cid = (
        run(
            [
                "docker",
                "run",
                "-d",
                "--rm",
                "--name",
                name,
                "--tmpfs",
                "/var/lib/mysql",
                "-p",
                "127.0.0.1::3306",
                "-v",
                str(tls) + ":/fixture-tls:ro",
                "-v",
                str(admin) + ":/fixture-admin:ro",
                "-e",
                "MYSQL_ROOT_PASSWORD_FILE=/fixture-tls/bootstrap-password",
                IMAGE,
                "--mysqlx=OFF",
                "--require-secure-transport=ON",
                "--ssl-ca=/fixture-tls/ca.pem",
                "--ssl-cert=/fixture-tls/server.pem",
                "--ssl-key=/fixture-tls/server.key",
                "--local-infile=OFF",
            ],
            timeout=300,
        )
        .decode()
        .strip()
    )

    try:
        port = int(
            run(["docker", "port", cid, "3306/tcp"]).decode().strip().split(":")[-1]
        )
        prefix = ["docker", "exec", "-i", cid, "mysql"]

        def reader(target):
            return [
                *prefix,
                "--defaults-extra-file=/fixture-admin/" + target + ".cnf",
                "--protocol=tcp",
                "-hlocalhost",
                "--ssl-mode=VERIFY_IDENTITY",
                "--ssl-ca=/fixture-tls/ca.pem",
                "-NB",
            ]

        yield port, [
            *prefix,
            "--defaults-extra-file=/fixture-admin/client.cnf",
            "-NB",
        ], reader
    finally:
        run(["docker", "rm", "-f", cid], timeout=60)


@contextlib.contextmanager
def native_server(base, mysql_bin, password):
    # Hosted macOS uses a private MySQL instance; no launch agent, shared datadir or default socket.
    datadir, tls, admin = base / "mysqldata", base / "tls", base / "admin"
    datadir.mkdir(mode=0o700)
    sock = str(base / "mysql.sock")
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    server = [
        str(mysql_bin / "mysqld"),
        "--no-defaults",
        "--basedir=" + str(mysql_bin.parent),
        "--datadir=" + str(datadir),
        "--log-error=" + str(base / "mysqld.log"),
        "--secure-file-priv=NULL",
    ]
    run([*server, "--initialize-insecure"], timeout=120)
    # MySQL applies the generated root password before accepting any connections.
    init = admin / "init.sql"
    init.write_text(f"ALTER USER 'root'@'localhost' IDENTIFIED BY '{password}';\n")
    init.chmod(0o400)
    proc = subprocess.Popen(
        [
            *server,
            "--init-file=" + str(init),
            "--socket=" + sock,
            "--port=" + str(port),
            "--bind-address=127.0.0.1",
            "--mysqlx=OFF",
            "--local-infile=OFF",
            "--require-secure-transport=ON",
            "--ssl-ca=" + str(tls / "ca.pem"),
            "--ssl-cert=" + str(tls / "server.pem"),
            "--ssl-key=" + str(tls / "server.key"),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        mysql = str(mysql_bin / "mysql")

        def reader(target):
            return [
                mysql,
                "--defaults-file=" + str(admin / (target + ".cnf")),
                "--protocol=tcp",
                "-hlocalhost",
                "--port=" + str(port),
                "--ssl-mode=VERIFY_IDENTITY",
                "--ssl-ca=" + str(tls / "ca.pem"),
                "-NB",
            ]

        yield port, [
            mysql,
            "--defaults-file=" + str(admin / "client.cnf"),
            "--socket=" + sock,
            "-NB",
        ], reader
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
            raise


@contextlib.contextmanager
def mysql_fixture(base, mysql_bin=None):
    tls = base / "tls"
    tls.mkdir(mode=0o755)

    def openssl(*args):
        run(["openssl", *args], cwd=tls)

    openssl(
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-days",
        "2",
        "-subj",
        "/CN=Envlume fixture CA",
        "-keyout",
        "ca.key",
        "-out",
        "ca.pem",
    )
    openssl(
        "req",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-subj",
        "/CN=localhost",
        "-keyout",
        "server.key",
        "-out",
        "server.csr",
    )
    (tls / "extensions").write_text(
        "subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n"
    )
    openssl(
        "x509",
        "-req",
        "-in",
        "server.csr",
        "-CA",
        "ca.pem",
        "-CAkey",
        "ca.key",
        "-CAcreateserial",
        "-days",
        "2",
        "-extfile",
        "extensions",
        "-out",
        "server.pem",
    )
    # Only the disposable server key and public certificates enter the container.
    (tls / "ca.key").unlink()
    (tls / "server.key").chmod(0o444)
    admin = base / "admin"
    admin.mkdir(mode=0o700)
    root = secrets.token_hex(32)
    (tls / "bootstrap-password").write_text(root)
    (tls / "bootstrap-password").chmod(0o444)
    (admin / "client.cnf").write_text(
        "[client]\nuser=root\npassword=" + root + "\nprotocol=socket\n"
    )
    for p in admin.iterdir():
        p.chmod(0o400)
    server = (
        docker_server(base)
        if mysql_bin is None
        else native_server(base, mysql_bin, root)
    )
    with server as (port, admin_cmd, reader_cmd):

        def sql(statement):
            return run(admin_cmd, data=statement.encode()).decode().strip()

        end = time.monotonic() + 120
        while True:
            try:
                if sql("SELECT @@port") == str(3306 if mysql_bin is None else port):
                    break
            except RuntimeError:
                pass
            if time.monotonic() > end:
                raise RuntimeError("fixture MySQL did not become ready")
            time.sleep(1)
        accounts = {}
        for target in ["dev", "test"]:
            db = "orders_" + target
            user = "t11_" + target
            pwd = secrets.token_hex(24)
            marker = target + "_marker"
            sql(f"""CREATE DATABASE {db};
            CREATE TABLE {db}.{marker}(id BIGINT PRIMARY KEY, name VARCHAR(64), hidden_value VARCHAR(32), payload MEDIUMTEXT,
                INDEX name_idx(name), INDEX unlisted_idx(name), INDEX mixed_idx(name,hidden_value));
            CREATE TABLE {db}.hidden_table(id INT PRIMARY KEY);
            CREATE TABLE {db}.db_hidden(id INT PRIMARY KEY);
            CREATE USER '{user}'@'%' IDENTIFIED BY '{pwd}' REQUIRE SSL;
            GRANT SELECT ON {db}.{marker} TO '{user}'@'%';
            GRANT SELECT ON {db}.hidden_table TO '{user}'@'%';""")
            sql(f"""INSERT INTO {db}.{marker}(id,name,payload)
                WITH RECURSIVE n AS (SELECT 1 AS i UNION ALL SELECT i+1 FROM n WHERE i<220)
                SELECT i,'synthetic-private-name',CONCAT('行',i) FROM n;
                UPDATE {db}.{marker} SET payload=REPEAT(CONCAT('界',CHAR(34),CHAR(92),CHAR(10)),500) WHERE id BETWEEN 20 AND 80;
                UPDATE {db}.{marker} SET payload=REPEAT('界',80000) WHERE id=220;""")
            grants = sql(f"SHOW GRANTS FOR '{user}'@'%'")
            assert all(
                line.startswith(("GRANT USAGE ON ", "GRANT SELECT ON "))
                for line in grants.splitlines()
            )
            client = admin / (target + ".cnf")
            client.write_text("[client]\nuser=" + user + "\npassword=" + pwd + "\n")
            client.chmod(0o400)
            mysql = reader_cmd(target) + [db]
            assert set(run(mysql, data=b"SHOW TABLES").decode().splitlines()) == {
                marker,
                "hidden_table",
            }
            denied = subprocess.run(
                mysql,
                input=f"INSERT INTO {marker}(id) VALUES(1)".encode(),
                capture_output=True,
            )
            assert denied.returncode and b"1142" in denied.stderr
            accounts[target] = {"user": user, "password": pwd}

        for target, other in [("dev", "test"), ("test", "dev")]:
            attempt = subprocess.run(
                reader_cmd(target),
                input=f"SELECT id FROM orders_{other}.{other}_marker LIMIT 1".encode(),
                capture_output=True,
            )
            assert attempt.returncode and b"1142" in attempt.stderr

        @contextlib.contextmanager
        def table_lock(*, both=False):
            lock = subprocess.Popen(
                [*admin_cmd, "--unbuffered"],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
            )
            try:
                tables = "orders_test.test_marker WRITE"
                if both:
                    tables += ", orders_dev.dev_marker WRITE"
                lock.stdin.write(f"LOCK TABLES {tables}; SELECT 1;\n".encode())
                lock.stdin.flush()
                assert lock.stdout.readline().strip() == b"1"
                yield
            finally:
                lock.stdin.close()
                assert lock.wait(timeout=10) == 0

        yield {
            "port": port,
            "ca": str(tls / "ca.pem"),
            "accounts": accounts,
        }, sql, table_lock


def binary(package, target, *flags):
    # Decode Cargo's artifact path, including when CARGO_TARGET_DIR or a target triple is set.
    build = subprocess.run(
        ["cargo", "build", "--locked", "--message-format=json", "-p", package, *flags],
        cwd=REPO,
        stdout=subprocess.PIPE,
    )
    data = build.stdout
    if build.returncode:
        for line in data.splitlines():
            item = json.loads(line)
            if item.get("reason") == "compiler-message":
                print(item["message"]["rendered"])
        raise RuntimeError("Cargo fixture build failed")
    return next(
        v["executable"]
        for line in data.splitlines()
        if (v := json.loads(line)).get("executable") and v["target"]["name"] == target
    )


def recv(proc, timeout=60, sizes=None):
    import select

    if not select.select([proc.stdout], [], [], timeout)[0]:
        raise RuntimeError("fixture response timed out")
    line = proc.stdout.readline()
    if not line:
        raise RuntimeError("fixture process exited")
    if sizes is not None:
        sizes.append(len(line))
    return json.loads(line)


def send(proc, msg):
    proc.stdin.write((json.dumps(msg) + "\n").encode())
    proc.stdin.flush()


def wait_for(check, description, seconds=12):
    end = time.monotonic() + seconds
    while not check():
        if time.monotonic() > end:
            raise AssertionError(description)
        time.sleep(0.1)


def review():
    import socket, sys

    exe = binary("envlume", "mysql_review", "--example", "mysql_review")
    # Using the same signed executable for provisioning and CLI/MCP preserves the native vault ACL.
    helper = exe if sys.platform == "darwin" else None
    with (
        tempfile.TemporaryDirectory(prefix="envlume-t11-") as tmp,
        socket.socket() as offline,
        socket.socket() as hung,
    ):
        base = Path(tmp)
        offline.bind(("127.0.0.1", 0))
        hung.bind(("127.0.0.1", 0))
        hung.listen(4)
        with mysql_fixture(base) as (cfg, sql, table_lock):
            cfg.update(
                base=tmp, offline=offline.getsockname()[1], hung=hung.getsockname()[1]
            )
            # On macOS a bound but non-listening socket can drop SYNs until timeout.
            # A closed local port exercises immediate refusal; the listener below exercises timeout.
            offline.close()
            proc = subprocess.Popen(
                [exe], cwd=base, stdin=subprocess.PIPE, stdout=subprocess.PIPE
            )

            def request(op, args=None):
                send(proc, {"op": op, "args": args})
                reply = recv(proc)
                wire = json.dumps(reply)
                assert all(
                    a["password"] not in wire and a["user"] not in wire
                    for a in cfg["accounts"].values()
                )
                assert "BEGIN CERTIFICATE" not in wire and "127.0.0.1" not in wire
                return reply

            def args(target="dev", resource="primary-db"):
                return {"target": target, "resource": resource}

            def connections(user):
                ids = sql(
                    "SELECT PROCESSLIST_ID FROM performance_schema.threads WHERE PROCESSLIST_USER='t11_"
                    + user
                    + "' ORDER BY PROCESSLIST_ID"
                )
                return set(ids.splitlines())

            try:
                send(proc, cfg)
                setup = recv(proc, 120)
                receipt = {
                    "mysql": sql("SELECT VERSION()"),
                    "tables": {},
                    "diagnostics": {},
                }
                foreign = request("foreign", args())
                assert (
                    foreign["reads"] == 0
                    and foreign["result"]["errorCode"] == "TARGET_NOT_ALLOWED"
                )
                receipt["deniedBeforeCredentials"] = [
                    "foreign",
                    "unapproved-address",
                    "dns-failure",
                ]
                for r in ["unapproved", "dns-fail"]:
                    reply = request("connection_test", args(resource=r))
                    assert not reply["result"]["ok"] and reply["reads"] == 0
                    assert reply["result"]["errorCode"] == (
                        "TARGET_NOT_ALLOWED"
                        if r == "unapproved"
                        else "NETWORK_UNREACHABLE"
                    )
                first = request("connection_test", args())["result"]
                assert first["ok"] and first["result"]["versionSummary"].startswith(
                    "MySQL 8.4."
                )
                ids = connections("dev")
                assert len(ids) == 1
                for _ in range(3):
                    assert request("connection_test", args())["result"]["ok"]
                assert connections("dev") == ids
                request("revoke")
                before = request("health")["reads"]
                refused = request("connection_test", args())
                assert (
                    refused["result"]["errorCode"] == "POLICY_DENIED"
                    and refused["reads"] == before
                )
                wait_for(
                    lambda: not (connections("dev") & ids),
                    "retired grant kept its old connection",
                )
                request("regrant")
                assert request("connection_test", args())["result"]["ok"]
                assert connections("dev").isdisjoint(ids)
                receipt["connectionReused"] = True
                receipt["newGrantNewConnection"] = True
                for name in ["dev", "test"]:
                    r = request("database_schema", args(name))["result"]
                    assert r["ok"], r
                    [table] = r["result"]["tables"]
                    receipt["tables"][name] = table
                    assert table["table"] == name + "_marker"
                    assert [c["name"] for c in table["columns"]] == ["id", "name"]
                    assert {i["name"]: i["columns"] for i in table["indexes"]} == {
                        "PRIMARY": ["id"],
                        "name_idx": ["name"],
                    }
                    denied = request(
                        "database_schema", {**args(name), "table": "hidden_table"}
                    )["result"]
                    assert denied["errorCode"] == "TARGET_NOT_ALLOWED"
                # Four requests overlap. Both grants have separate bounded pools.
                rows = request(
                    "parallel", [args("dev"), args("test"), args("dev"), args("test")]
                )["result"]
                assert [r["result"]["tables"][0]["table"] for r in rows] == [
                    "dev_marker",
                    "test_marker",
                ] * 2
                assert (
                    1 <= len(connections("dev")) <= 3
                    and 1 <= len(connections("test")) <= 2
                )
                receipt["parallelConnections"] = {
                    n: len(connections(n)) for n in ["dev", "test"]
                }
                for r, code, reason in [
                    ("bad-cert", "NETWORK_UNREACHABLE", "TLS"),
                    ("wrong-name", "NETWORK_UNREACHABLE", "TLS"),
                    ("bad-login", "CREDENTIAL_UNAVAILABLE", "login"),
                    ("offline", "NETWORK_UNREACHABLE", "TCP"),
                    ("hung", "TIMEOUT", "deadline"),
                ]:
                    start = time.monotonic()
                    result = request("connection_test", args(resource=r))["result"]
                    assert (
                        not result["ok"]
                        and result["errorCode"] == code
                        and reason in result["summary"]
                    ), (r, result)
                    receipt["diagnostics"][r] = {
                        "code": code,
                        "summary": result["summary"],
                        "durationMs": result["durationMs"],
                    }
                    assert time.monotonic() - start < 10
                health = request("health")["result"]["result"]["environments"]
                statuses = {
                    r["alias"]: r["health"]
                    for e in health
                    if e["target"] == "dev"
                    for r in e["resources"]
                }
                assert (
                    statuses["primary-db"] == "healthy"
                    and statuses["bad-login"] == "unavailable"
                )
                receipt["query"] = check_queries(request, sql, table_lock)
                if helper:
                    receipt["cliAudit"] = check_cli_mcp(helper, setup)
                assert request("connection_test", args())["result"]["ok"]

                # The heartbeat only reads local state: it cannot open or keep a MySQL session alive.
                def expired():
                    if connections("dev") or connections("test"):
                        return False
                    idle = request("health")["result"]["result"]["environments"]
                    return all(
                        r["health"] == "unknown" for e in idle for r in e["resources"]
                    )

                wait_for(expired, "idle connections or cached health exceeded TTL", 40)
                receipt["idleConnections"] = 0
                assert request("connection_test", args("test"))["result"]["ok"]
                assert len(connections("test")) == 1
                request("close")
                wait_for(
                    lambda: not connections("dev") and not connections("test"),
                    "physical MySQL sessions not closed",
                )
                receipt["connectionsAfterClose"] = {
                    n: len(connections(n)) for n in ["dev", "test"]
                }
                print("T11 evidence " + json.dumps(receipt, sort_keys=True), flush=True)
                print(
                    "T11: MySQL",
                    sql("SELECT VERSION()"),
                    "DEV/TEST read-only grants, reuse, concurrent metadata, hidden table/column/index, DNS/IP, TLS CA/identity, authentication, TCP, timeout, revocation and close passed.",
                    flush=True,
                )
            finally:
                proc.stdin.close()
                try:
                    rc = proc.wait(timeout=25)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
                    raise
                assert rc == 0, "fixture teardown failed"


def check_queries(request, sql, table_lock):
    def args(text, target="dev"):
        return {"target": target, "resource": "query-db", "sql": text}

    def query(text, target="dev"):
        report = request("database_query", args(text, target))["result"]
        assert report["ok"], report
        return report

    baseline = query(
        "SELECT a.id, a.name AS customer FROM orders_dev.dev_marker a ORDER BY a.id LIMIT 1"
    )
    assert baseline["evidenceId"] and baseline["result"] == {
        "columns": ["id", "customer"],
        "rows": [[1, "[REDACTED]"]],
        "truncated": False,
    }
    nested = query(
        "SELECT a.* FROM (SELECT name AS customer, id FROM dev_marker WHERE id = 1) AS a"
    )
    assert nested["result"]["rows"] == [["[REDACTED]", 1]]
    assert query("SELECT COUNT(*) AS n FROM dev_marker")["result"]["rows"] == [[220]]
    assert query("SELECT id FROM dev_marker WHERE id = 0")["result"]["rows"] == []

    def statements():
        return sql(
            "SELECT COALESCE(SUM(COUNT_STAR),0) FROM performance_schema.events_statements_summary_by_account_by_event_name WHERE USER='t11_dev'"
        )

    counter = statements()
    before = request("health")["reads"]
    denied = [
        "SELECT id FROM dev_marker; SELECT id FROM dev_marker",
        "SELECT SLEEP(0) FROM dev_marker",
        "SELECT id FROM dev_marker FOR UPDATE",
        "DELETE FROM dev_marker WHERE id = 0",
        "SHOW TABLES",
        "SELECT id FROM orders_test.test_marker",
        "SELECT id FROM hidden_table",
        "SELECT hidden_value FROM dev_marker",
        "SELECT d.* FROM (SELECT hidden_value FROM dev_marker) d",
        "SELECT other.* FROM dev_marker",
        "SELECT id FROM dev_marker ORDER BY name",
        "SELECT id FROM dev_marker WHERE name = 'test'",
    ]
    for text in denied:
        refusal = request("database_query", args(text))
        assert (
            refusal["reads"] == before
            and refusal["result"]["errorCode"] == "QUERY_NOT_ALLOWED"
        ), refusal
    assert statements() == counter
    assert (
        request(
            "database_query", {**args("SELECT id FROM dev_marker"), "maxRows": 999}
        )["result"]["errorCode"]
        == "INVALID_INPUT"
    )
    rows = query("SELECT id FROM dev_marker ORDER BY id LIMIT 99999")["result"]
    assert rows["truncated"] and rows["rows"] == [[i] for i in range(1, 201)]
    long = query("SELECT payload FROM dev_marker WHERE id = 220")["result"]
    assert long["truncated"] and not long["rows"]
    bounded = query("SELECT payload FROM dev_marker WHERE id >= 20 ORDER BY id")[
        "result"
    ]
    assert bounded["truncated"] and 0 < len(bounded["rows"]) < 61
    assert bounded["rows"][0][0] == ("界" + chr(34) + chr(92) + chr(10)) * 500
    assert (
        len(query("SELECT id FROM test_marker LIMIT 1000", "test")["result"]["rows"])
        == 7
    )
    request("tighten")
    assert (
        request("database_query", args("SELECT id FROM test_marker", "test"))["result"][
            "errorCode"
        ]
        == "POLICY_DENIED"
    )
    request("regrant-read")
    assert (
        len(query("SELECT id FROM test_marker LIMIT 1000", "test")["result"]["rows"])
        == 3
    )

    sql("GRANT UPDATE ON orders_test.test_marker TO 't11_test'@'%'")
    try:
        broad = request("database_query", args("SELECT id FROM test_marker", "test"))[
            "result"
        ]
        assert broad["errorCode"] == "POLICY_DENIED" and "read-only" in broad["summary"]
    finally:
        sql("REVOKE UPDATE ON orders_test.test_marker FROM 't11_test'@'%'")
    with table_lock():
        start = time.monotonic()
        reports = request(
            "parallel-query",
            [args("SELECT id FROM test_marker", "test") for _ in range(12)],
        )["result"]
        elapsed = time.monotonic() - start
        assert all(r.get("errorCode") == "TIMEOUT" for r in reports), [
            (r.get("errorCode"), r["summary"], r["durationMs"]) for r in reports
        ]
        assert any("pool limit" in r["summary"] for r in reports)
        assert any("deadline" in r["summary"] for r in reports)
        assert elapsed < 5
        wait_for(
            lambda: sql(
                "SELECT COUNT(*) FROM performance_schema.threads WHERE PROCESSLIST_USER='t11_test' AND PROCESSLIST_STATE='Waiting for table metadata lock'"
            )
            == "0",
            "timed-out query still waiting on server metadata lock",
            4,
        )
    assert query("SELECT id FROM test_marker WHERE id = 1", "test")["result"][
        "rows"
    ] == [[1]]
    return {
        "deniedBeforeLoginAndMySQL": len(denied),
        "defaultRows": len(rows["rows"]),
        "escapedRows": len(bounded["rows"]),
        "tightenedRows": 3,
        "parallel": len(reports),
        "elapsedMs": round(elapsed * 1000),
    }


def check_cli_mcp(helper, setup):
    env = {**os.environ, "T05_FIXTURE_STORE": setup["store"]}
    scope = ["--binding", setup["binding"]]

    def cli(*args):
        return json.loads(
            run([helper, *args, *scope, "--json"], cwd=setup["workspace"], env=env)
        )

    checked = cli("connection", "test", "dev", "primary-db")
    audit = cli("report", "show", "--evidence", checked["evidenceId"])
    assert audit["requestId"] == checked["requestId"] and audit["targetAlias"] == "dev"
    assert (
        audit["connection"] == checked["result"]
        and audit["resourceId"]
        and audit["targetId"]
    )
    assert (
        cli("database", "schema", "test", "primary-db", "--table", "test_marker")[
            "result"
        ]["tables"][0]["table"]
        == "test_marker"
    )
    proc = subprocess.Popen(
        [helper, "mcp", *scope],
        cwd=setup["workspace"],
        env=env,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    seq = 0

    sizes = []

    def call(method, params):
        nonlocal seq
        seq += 1
        send(proc, {"jsonrpc": "2.0", "id": seq, "method": method, "params": params})
        msg = recv(proc, sizes=sizes)
        assert msg["id"] == seq
        return msg["result"]

    try:
        call(
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "t11", "version": "1"},
            },
        )
        send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized"})

        def tool(name, arguments):
            return call("tools/call", {"name": name, "arguments": arguments})[
                "structuredContent"
            ]

        ctx = tool("project_context", {})
        assert ctx["ok"] and ctx["result"]["targets"] == ["dev", "test"]
        advertised = call("tools/list", {})["tools"]
        frozen = json.loads((REPO / "tests/contracts/tools.json").read_text())
        for name in ["connection_test", "database_schema", "database_query"]:
            assert next(
                t["inputSchema"] for t in advertised if t["name"] == name
            ) == next(t["inputSchema"] for t in frozen if t["name"] == name)
        result = tool("database_schema", {"target": "dev", "resource": "primary-db"})
        assert result["result"]["tables"][0]["table"] == "dev_marker"
        reader = {"target": "dev", "resource": "query-db"}
        query_text = "SELECT name,payload FROM dev_marker WHERE id >= 20 ORDER BY id"
        cli_wire = run(
            [helper, "database", "query", "dev", "query-db", *scope, "--json"],
            cwd=setup["workspace"],
            env=env,
            data=query_text.encode(),
        )
        cli_result = json.loads(cli_wire)
        assert (
            cli_result["ok"]
            and cli_result["result"]["truncated"]
            and len(cli_wire) <= 262144
        )
        report = tool("database_query", {**reader, "sql": query_text})
        assert (
            report["result"] == cli_result["result"]
            and sizes[-1] <= 262144
            and report["evidenceId"]
        )
        full_bytes = sizes[-1]
        assert all(row[0] == "[REDACTED]" for row in report["result"]["rows"])
        audit = cli("report", "show", "--evidence", report["evidenceId"])
        assert audit["query"] == {
            "rowCount": len(report["result"]["rows"]),
            "columnCount": 2,
            "maskedColumns": 1,
            "truncated": True,
        }
        assert (
            audit["requestId"] == report["requestId"]
            and audit["targetId"]
            and audit["resourceId"]
        )
        assert (
            "rows" not in audit
            and "result" not in audit
            and len(json.dumps(audit)) < 4096
        )
        assert all(
            s not in json.dumps(audit, ensure_ascii=False)
            for s in [query_text, "synthetic-private-name", "界", "[REDACTED]"]
        )
        small = tool(
            "database_query",
            {
                "target": "test",
                "resource": "query-db",
                "sql": "SELECT payload FROM test_marker WHERE id >= 20 ORDER BY id",
            },
        )
        assert small["result"]["truncated"] and sizes[-1] <= 8192
        print(
            "T12 wire evidence "
            + json.dumps(
                {
                    "cliBytes": len(cli_wire),
                    "mcpBytes": full_bytes,
                    "tightMcpBytes": sizes[-1],
                    "evidenceId": report["evidenceId"],
                    "requestId": report["requestId"],
                    "audit": audit["query"],
                }
            ),
            flush=True,
        )
        denied = tool(
            "connection_test",
            {"target": "dev", "resource": "primary-db", "host": "local.invalid"},
        )
        assert denied["errorCode"] == "INVALID_INPUT"
        print(
            "T11: independent CLI, live MCP, frozen schemas and binding-scoped audit lookup passed.",
            flush=True,
        )
    finally:
        proc.stdin.close()
        assert proc.wait(timeout=20) == 0
    return {k: checked[k] for k in ["requestId", "evidenceId", "result"]}


if __name__ == "__main__":
    review()
