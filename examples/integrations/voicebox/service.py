#!/usr/bin/env python3
"""voicebox — CO-503 / CO-522 / CO-531 predictive HTTP voice tool stub.

A minimal Python *stdlib-only* HTTP service that mirrors the contract a real
self-hosted voicebox (jamiepine/voicebox) STT/TTS deployment would expose:

    POST /voice  {"action": "transcribe", "audio_b64": "..."}
                 -> {"text": "<transcript>"}
    POST /voice  {"action": "synthesize", "text": "...", "voice": "..."}
                 -> {"audio_b64": "<bytes>", "note": "stub"}
    GET  /health -> {"status": "ok"}

It returns canned demo output so a reviewer can curl it offline. It NEVER calls a
cloud STT/TTS provider — local-first by construction. Swapping in the real voicebox
backend means replacing `transcribe()` / `synthesize()` (the backend is injected /
configured); the HTTP contract (and the CO manifest) stay identical. That is the
whole point: the agent calls this like any other tool and never knows what runs
behind the URL.

Run:  python3 service.py            # listens on 127.0.0.1:9200
"""

import base64
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HOST = "127.0.0.1"
PORT = 9200

# A canned transcript so a reviewer sees a stable transcribe result offline.
DEMO_TRANSCRIPT = (
    "Olá, isto é uma transcrição de demonstração gerada localmente pelo stub do "
    "voicebox — nenhum áudio saiu da instância."
)
PLACEHOLDER_TRANSCRIPT = "[no audio_b64 supplied — placeholder transcript]"


def transcribe(audio_b64):
    """STT stub. A real voicebox backend would decode + run local whisper here.

    Local-first: audio is never forwarded to a cloud provider. If no audio was
    supplied we echo a placeholder so the contract is still demonstrable.
    """
    if not isinstance(audio_b64, str) or not audio_b64.strip():
        return {"text": PLACEHOLDER_TRANSCRIPT}
    return {"text": DEMO_TRANSCRIPT}


def synthesize(text, voice):
    """TTS stub. A real voicebox backend would render audio locally and return it.

    Here we just emit a few fake bytes (base64) so the JSON shape is real.
    """
    fake_audio = f"VOICEBOX-STUB::{voice or 'default'}::{text}".encode("utf-8")
    return {
        "audio_b64": base64.b64encode(fake_audio).decode("ascii"),
        "note": "stub",
    }


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, payload):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.rstrip("/") == "/health":
            self._send(200, {"status": "ok", "actions": ["transcribe", "synthesize"]})
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        if self.path.rstrip("/") != "/voice":
            self._send(404, {"error": "not found"})
            return
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            args = json.loads(raw or b"{}")
        except json.JSONDecodeError:
            self._send(400, {"error": "invalid JSON body"})
            return
        action = args.get("action")
        if action == "transcribe":
            self._send(200, transcribe(args.get("audio_b64")))
        elif action == "synthesize":
            text = args.get("text")
            if not isinstance(text, str) or not text.strip():
                self._send(400, {"error": "`text` (string) is required for synthesize"})
                return
            self._send(200, synthesize(text, args.get("voice")))
        else:
            self._send(
                400,
                {"error": "`action` must be 'transcribe' or 'synthesize'"},
            )

    def log_message(self, *_args):
        pass  # keep stdout clean for reviewers


def main():
    srv = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"voicebox stub listening on http://{HOST}:{PORT}  "
          f"(POST /voice, GET /health) — Ctrl-C to stop")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print("\nshutting down")
        srv.shutdown()


if __name__ == "__main__":
    main()
