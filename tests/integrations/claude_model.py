"""A loopback Messages fixture: request one advertised tool, then acknowledge its result."""

import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Thread
from contextlib import contextmanager

TOOL = "mcp__envlume__project_context"


@contextmanager
def model_fixture():
    observed = {"advertised": False, "toolResults": []}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            if self.path.split("?")[0] != "/v1/messages":
                self.send_error(404)
                return
            size = int(self.headers.get("Content-Length", "0"))
            if not 0 < size <= 4 * 1024 * 1024:
                self.send_error(413)
                return
            body = json.loads(self.rfile.read(size))
            tools = {t["name"] for t in body.get("tools", []) if "name" in t}
            result = [
                b
                for msg in body["messages"]
                if isinstance(msg["content"], list)
                for b in msg["content"]
                if b.get("type") == "tool_result"
                and b.get("tool_use_id") == "toolu_t16"
            ]
            if result:
                observed["toolResults"].extend(result)
            call = TOOL in tools and not result
            observed["advertised"] |= TOOL in tools
            block = (
                {"type": "tool_use", "id": "toolu_t16", "name": TOOL, "input": {}}
                if call
                else {"type": "text", "text": "T16 fixture complete."}
            )
            reason = "tool_use" if call else "end_turn"
            msg = {
                "type": "message",
                "role": "assistant",
                "id": "msg_t16",
                "model": body["model"],
                "content": [block],
                "stop_reason": reason,
                "stop_sequence": None,
                "usage": {"input_tokens": 20, "output_tokens": 12},
            }
            self.send_response(200)
            if not body.get("stream"):
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps(msg).encode())
                return
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()

            def event(kind, **data):
                payload = json.dumps({"type": kind, **data})
                self.wfile.write(f"event: {kind}\ndata: {payload}\n\n".encode())

            event("message_start", message={**msg, "content": [], "stop_reason": None})
            event(
                "content_block_start",
                index=0,
                content_block=block if call else {"type": "text", "text": ""},
            )
            delta = (
                {"type": "input_json_delta", "partial_json": "{}"}
                if call
                else {"type": "text_delta", "text": block["text"]}
            )
            event("content_block_delta", index=0, delta=delta)
            event("content_block_stop", index=0)
            event(
                "message_delta",
                delta={"stop_reason": reason, "stop_sequence": None},
                usage={"output_tokens": 12},
            )
            event("message_stop")

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}", observed
        finally:
            server.shutdown()
            thread.join()
