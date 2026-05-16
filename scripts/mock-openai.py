#!/usr/bin/env python3
import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/v1/models":
            self._json(
                200,
                {
                    "object": "list",
                    "data": [{"id": "mock-gpt", "object": "model", "owned_by": "mizan"}],
                },
            )
            return
        self._json(404, {"error": {"message": "not found"}})

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self._json(404, {"error": {"message": "not found"}})
            return

        length = int(self.headers.get("content-length", "0"))
        payload = json.loads(self.rfile.read(length) or b"{}")
        if payload.get("stream"):
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.end_headers()
            chunk = {
                "id": "chatcmpl-mock",
                "object": "chat.completion.chunk",
                "model": payload.get("model", "mock-gpt"),
                "choices": [
                    {"index": 0, "delta": {"content": "hello from mock"}, "finish_reason": None}
                ],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 4,
                    "total_tokens": 7,
                },
            }
            self.wfile.write(f"data: {json.dumps(chunk)}\n\n".encode())
            self.wfile.write(b"data: [DONE]\n\n")
            return

        self._json(
            200,
            {
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "model": payload.get("model", "mock-gpt"),
                "choices": [
                    {
                        "index": 0,
                        "message": {"role": "assistant", "content": "hello from mock"},
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 4,
                    "total_tokens": 7,
                },
            },
        )

    def log_message(self, format, *args):
        return

    def _json(self, status, payload):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=18182)
    args = parser.parse_args()
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"mock OpenAI-compatible upstream listening on {args.host}:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
