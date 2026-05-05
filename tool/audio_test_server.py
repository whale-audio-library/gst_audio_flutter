#!/usr/bin/env python3
"""HTTP server with deterministic slow and interrupted responses for tests."""

from __future__ import annotations

import argparse
import http.server
import mimetypes
import socket
import socketserver
import time
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse


class ThreadingServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


class AudioTestHandler(http.server.SimpleHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def handle_one_request(self) -> None:
        try:
            super().handle_one_request()
        except (BrokenPipeError, ConnectionResetError, socket.timeout):
            self.close_connection = True

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        query = parse_qs(parsed.query)

        if parsed.path.startswith("/drop/"):
            self._send_audio(parsed.path.removeprefix("/drop/"), query, drop_after_bytes=16384)
            return

        if parsed.path.startswith("/slow/"):
            self._send_audio(parsed.path.removeprefix("/slow/"), query, sleep_seconds=0.035)
            return

        self._send_audio(parsed.path.removeprefix("/"), query)

    def do_HEAD(self) -> None:
        parsed = urlparse(self.path)
        if parsed.path.startswith("/drop/") or parsed.path.startswith("/slow/"):
            self._send_audio(parsed.path.split("/", 2)[2], parse_qs(parsed.query), head_only=True)
            return
        self._send_audio(parsed.path.removeprefix("/"), parse_qs(parsed.query), head_only=True)

    def _send_audio(
        self,
        relative_path: str,
        query: dict[str, list[str]],
        *,
        sleep_seconds: float = 0.0,
        drop_after_bytes: int | None = None,
        head_only: bool = False,
    ) -> None:
        path = self.translate_path("/" + unquote(relative_path))
        file_path = Path(path)
        if not file_path.is_file():
            self.send_error(404, "File not found")
            return

        chunk_size = _int_query(query, "chunk", 2048)
        delay = _float_query(query, "delay", sleep_seconds)
        content_type = mimetypes.guess_type(file_path.name)[0] or "application/octet-stream"
        size = file_path.stat().st_size
        range_result = self._parse_range(size)
        if range_result is None:
            return
        start, end = range_result
        response_size = end - start + 1

        self.send_response(206 if start > 0 or end < size - 1 else 200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(response_size))
        self.send_header("Accept-Ranges", "bytes")
        if start > 0 or end < size - 1:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.end_headers()

        if head_only:
            return

        sent = 0
        with file_path.open("rb") as file:
            file.seek(start)
            while True:
                remaining = response_size - sent
                if remaining <= 0:
                    break
                chunk = file.read(min(max(1, chunk_size), remaining))
                if not chunk:
                    break
                if drop_after_bytes is not None and sent >= drop_after_bytes:
                    self.close_connection = True
                    return
                if drop_after_bytes is not None and sent + len(chunk) > drop_after_bytes:
                    chunk = chunk[: drop_after_bytes - sent]
                self.wfile.write(chunk)
                self.wfile.flush()
                sent += len(chunk)
                if delay > 0:
                    time.sleep(delay)

    def _parse_range(self, size: int) -> tuple[int, int] | None:
        header = self.headers.get("Range", "")
        if not header.startswith("bytes="):
            return 0, size - 1

        spec = header.removeprefix("bytes=").split(",", 1)[0].strip()
        if "-" not in spec:
            return 0, size - 1
        start_text, end_text = spec.split("-", 1)
        try:
            if start_text:
                start = int(start_text)
                end = int(end_text) if end_text else size - 1
            else:
                suffix = int(end_text)
                start = max(0, size - suffix)
                end = size - 1
        except ValueError:
            return 0, size - 1

        if start < 0 or start >= size or end < start:
            self.send_error(416, "Requested Range Not Satisfiable")
            return None
        return start, min(end, size - 1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--directory", default="test-assets")
    args = parser.parse_args()

    handler = lambda *handler_args: AudioTestHandler(  # noqa: E731
        *handler_args,
        directory=args.directory,
    )
    with ThreadingServer((args.host, args.port), handler) as server:
        print(f"Serving {args.directory} on http://{args.host}:{args.port}/", flush=True)
        server.serve_forever()


def _int_query(query: dict[str, list[str]], key: str, fallback: int) -> int:
    try:
        return int(query.get(key, [fallback])[0])
    except (TypeError, ValueError):
        return fallback


def _float_query(query: dict[str, list[str]], key: str, fallback: float) -> float:
    try:
        return float(query.get(key, [fallback])[0])
    except (TypeError, ValueError):
        return fallback


if __name__ == "__main__":
    main()
