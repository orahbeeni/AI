#!/usr/bin/env python3
"""Serve txted on http://localhost:8080 (localhost counts as a secure context, so the PWA + file APIs work)."""
import http.server, sys
port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
http.server.test(HandlerClass=http.server.SimpleHTTPRequestHandler, port=port, bind="127.0.0.1")
