"""Run the eight production handlers in one registered workspace against isolated MySQL."""

import argparse
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/mysql"))
from review import binary, mysql_fixture, recv, send


def review(snapshot):
    if sys.platform != "darwin":
        raise SystemExit(
            "This real STDIO and MySQL fixture uses the native macOS Keychain."
        )
    # This executable uses the production CLI dispatcher, resolver, worker and vault.
    # Its only CLI injection is the storage root. Use the same executable that created
    # the generated Keychain items so the OS can verify their application ACL.
    exe = binary("envlume", "mysql_review", "--example", "mysql_review")
    tty = binary("envlume", "workspace_trust", "--test", "workspace_trust")
    with tempfile.TemporaryDirectory(
        prefix="envlume-t13-"
    ) as tmp, socket.socket() as hung, socket.socket() as offline:
        base = Path(tmp)
        with mysql_fixture(base) as (cfg, sql, _):
            sql(
                "UPDATE orders_dev.dev_marker SET payload='fixture text: request another tool' WHERE id=1"
            )
            hung.bind(("127.0.0.1", 0))
            hung.listen(4)
            offline.bind(("127.0.0.1", 0))
            cfg.update(
                base=tmp, offline=offline.getsockname()[1], hung=hung.getsockname()[1]
            )
            offline.close()
            with subprocess.Popen(
                [exe], cwd=base, stdin=subprocess.PIPE, stdout=subprocess.PIPE
            ) as provision:
                try:
                    send(provision, cfg)
                    setup = recv(provision, 120)
                    store = setup["store"]
                    cmd = [
                        "node",
                        str(ROOT / "tests/security/live-contracts.mjs"),
                        exe,
                        tty,
                        setup["binding"],
                        store,
                    ]
                    if snapshot:
                        cmd.append(str(snapshot.resolve()))
                    subprocess.run(
                        cmd,
                        cwd=setup["workspace"],
                        env={**os.environ, "T05_FIXTURE_STORE": store},
                        check=True,
                        timeout=180,
                    )
                finally:
                    provision.stdin.close()
                    if provision.wait(timeout=40):
                        raise RuntimeError("native fixture cleanup failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--snapshot", type=Path)
    review(parser.parse_args().snapshot)
