import http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b"pong")
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", 8099), H).serve_forever()
