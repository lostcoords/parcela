"""Disposable HTML app for the dashboard's browser and response-preservation checks."""
import gzip
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/ready':
            body, content_type = b'ready', 'text/plain'
        elif self.path == '/asset.js':
            body, content_type = b'document.body.dataset.appReady = "yes";', 'text/javascript'
        else:
            body = (f'<!doctype html><html><head><title>Disposable app</title></head><body>'
                    f'<h1>Local application</h1><a href="/second">Relative navigation</a>'
                    f'<script src="/asset.js"></script>'
                    f'<script defer src="{os.environ["TEST_UI_ORIGIN"]}/widget.js"></script>'
                    f'</body></html>').encode()
            content_type = 'text/html'
        self.send_response(200)
        self.send_header('Content-Type', content_type)
        if self.path == '/strict':
            self.send_header('Content-Security-Policy', "default-src 'none'; script-src 'none'")
        if self.path == '/gzip':
            body = gzip.compress(body)
            self.send_header('Content-Encoding', 'gzip')
        if os.environ.get('BLOCK_FRAME') == '1':
            self.send_header('X-Frame-Options', 'DENY')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


ThreadingHTTPServer(('127.0.0.1', int(os.environ['PORT'])), Handler).serve_forever()
