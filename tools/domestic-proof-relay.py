#!/usr/bin/env python3
"""Forward one domestic identity confirmation through a loopback relay."""

import http.server
import pathlib
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def serve(port, node_id, etc):
    relay_id = str(uuid.UUID((etc / "node-id").read_text().strip()))
    token = (etc / "node-token").read_text().strip()
    api = (etc / "api-url").read_text().strip().rstrip("/")
    parsed = urllib.parse.urlsplit(api)
    if (parsed.scheme != "https" or not parsed.hostname or parsed.username
            or parsed.password or parsed.query or parsed.fragment or not token.startswith(relay_id + ".")):
        raise ValueError("invalid local relay credentials")
    url = f"{api}/v1/nodes/{relay_id}/domestic-enroll/{node_id}"
    client = urllib.request.build_opener(NoRedirect())

    class Relay(http.server.BaseHTTPRequestHandler):
        confirmed = False

        def setup(self):
            self.request.settimeout(8)
            super().setup()

        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                length = 0
            if self.path != "/confirm" or not 1 <= length <= 2048:
                self.send_error(400)
                return
            request = urllib.request.Request(url, data=self.rfile.read(length), headers={
                "Content-Type": "application/json", "Authorization": "Bearer " + token,
                "User-Agent": "tunnel-agent-domestic-proof/1",
            })
            try:
                with client.open(request, timeout=10) as response:
                    self.reply(response.status, response.read(2048))
                Relay.confirmed = True
            except urllib.error.HTTPError as error:
                self.reply(error.code, error.read(2048))
            except urllib.error.URLError:
                self.send_error(502)

        def reply(self, status, data):
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    with http.server.HTTPServer(("127.0.0.1", port), Relay) as server:
        server.timeout = 5
        for _ in range(36):
            server.handle_request()
            if Relay.confirmed:
                return
    raise SystemExit("relay expired without confirmation")


if __name__ == "__main__":
    serve(int(sys.argv[1]), str(uuid.UUID(sys.argv[2])), pathlib.Path("/etc/tunnel-agent"))
