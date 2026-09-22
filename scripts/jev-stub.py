#!/usr/bin/env python3
"""A stub System One endpoint, so `clank-jev` can be exercised with no key and
no network.

Binds 127.0.0.1 (an ephemeral port unless --port names one), answers every POST
with one fixed JSON body, and writes the URL it landed on to --url-file once it
is listening. Same idea as the `Stub` in tests/jev.rs: plain HTTP/1.1 with a
JSON body, no SSE, so the CLI is driven through its real wire protocol rather
than a shortcut inside the client.
"""
import argparse
import http.server
import json
import pathlib
import sys


def handler(reply: bytes, status: int):
    class Stub(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_POST(self):  # noqa: N802 — the stdlib names this one
            length = int(self.headers.get("Content-Length", "0"))
            self.rfile.read(length)  # drained; the reply is fixed, not computed
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(reply)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(reply)

        def log_message(self, *_args):
            pass  # a request log would compete with the URL on stdout

    return Stub


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reply", type=pathlib.Path, required=True,
                        help="file holding the JSON body to answer every request with")
    parser.add_argument("--url-file", type=pathlib.Path, required=True,
                        help="the listening URL is written here, one line, before the first request")
    parser.add_argument("--port", type=int, default=0,
                        help="port to bind on 127.0.0.1 (default 0, an ephemeral one)")
    parser.add_argument("--status", type=int, default=200,
                        help="HTTP status to answer with (default 200)")
    args = parser.parse_args(argv)

    reply = args.reply.read_bytes()
    json.loads(reply)  # a malformed reply is a broken gate, not a stub-side surprise

    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), handler(reply, args.status))
    url = f"http://127.0.0.1:{server.server_address[1]}/v1/systemone"
    args.url_file.write_text(url + "\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
