#!/usr/bin/env bash
# whatsapp-webhook-smoke — Layer-4 integration test: Meta → co-web → brain,
# locally, WITHOUT touching Meta. Proves the whole inbound path:
#   • POST a correctly-signed (X-Hub-Signature-256 / WHATSAPP_APP_SECRET) Cloud
#     API payload to the local webhook → co-web verifies + returns a fast 200
#   • co-web forwards {text, sender, phone_number_id} to CO_BOT_BRAIN_URL — we
#     assert the tenant phone_number_id (1238594552665575) rides through (CO-489)
#   • a forged signature is rejected with 401 (the HMAC gate)
#   • the GET verification handshake echoes hub.challenge for the right token
#
# Self-contained: a temp co-web (CO_ENV=local) + a tiny capture "brain" that
# records what co-web forwards. The final Cloud-API send to Meta is out of scope
# here (that's Layer 6 — real token); WHATSAPP_CLOUD_TOKEN is set to a dummy only
# so the provider constructs and the brain actually gets called. Exit 0 = pass.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WEB="${CO_WEB_BIN:-$ROOT/target/debug/co-web}"
PORT="${SMOKE_PORT:-39590}"
BRAIN_PORT="${SMOKE_BRAIN_PORT:-39591}"
BASE="http://localhost:$PORT"
SECRET="smoke-app-secret"
VERIFY="smoke-verify-token"
PNID="1238594552665575"            # must match TENANT_REGISTRY + Meta metadata
DATA="$(mktemp -d /tmp/co-wa-smoke.XXXX)"
CAPTURE="$DATA/captured.json"
WEB_PID=""; BRAIN_PID=""
PASS=0; FAIL=0
ok(){ echo "  ✅ $1"; PASS=$((PASS+1)); }
bad(){ echo "  ❌ $1"; FAIL=$((FAIL+1)); }
cleanup(){ for p in "$WEB_PID" "$BRAIN_PID"; do [ -n "$p" ] && kill "$p" 2>/dev/null; done; rm -rf "$DATA"; }
trap cleanup EXIT

echo "== build co-web (if needed) =="
[ -x "$WEB" ] || cargo build -p co-web -q || { echo "build failed"; exit 2; }

echo "== capture brain on :$BRAIN_PORT (records what co-web forwards) =="
cat > "$DATA/brain.py" <<PY
import json, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
CAP = sys.argv[1]
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(n)
        open(CAP, "wb").write(raw)
        body = json.dumps({"reply": "ack de teste", "intent": "chat"}).encode()
        self.send_response(200); self.send_header("Content-Type","application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a): pass
ThreadingHTTPServer(("127.0.0.1", int(sys.argv[2])), H).serve_forever()
PY
python3 "$DATA/brain.py" "$CAPTURE" "$BRAIN_PORT" & BRAIN_PID=$!
disown "$BRAIN_PID" 2>/dev/null || true   # reap quietly on cleanup (no "Terminated" noise)

echo "== free + boot co-web on :$PORT (CO_ENV=local, WhatsApp webhook configured) =="
STRAY="$(lsof -ti :"$PORT" 2>/dev/null || true)"; [ -n "$STRAY" ] && { kill $STRAY 2>/dev/null; sleep 1; }
( cd "$ROOT" && CO_WEB_DATA="$DATA" CO_WEB_PORT="$PORT" JWT_SECRET="wa-smoke" CO_ENV="local" \
    WHATSAPP_APP_SECRET="$SECRET" WHATSAPP_VERIFY_TOKEN="$VERIFY" \
    WHATSAPP_CLOUD_TOKEN="dummy-token-for-smoke" WHATSAPP_PHONE_NUMBER_ID="$PNID" \
    CO_BOT_BRAIN_URL="http://127.0.0.1:$BRAIN_PORT/api/chat" \
    "$WEB" > "$DATA/server.log" 2>&1 ) & WEB_PID=$!
for i in $(seq 1 40); do curl -s --max-time 2 "$BASE/api/health" >/dev/null 2>&1 && break; sleep 1; done
curl -s --max-time 3 "$BASE/api/health" >/dev/null 2>&1 || { echo "co-web never came up"; tail -20 "$DATA/server.log"; exit 2; }
LISTENER="$(lsof -ti :"$PORT" 2>/dev/null | head -1)"; [ -n "$LISTENER" ] && WEB_PID="$LISTENER"

# A realistic Cloud API inbound payload (tenant id in metadata.phone_number_id).
# Fictional sender/display number — never a real person's number (public repo).
printf '%s' '{"object":"whatsapp_business_account","entry":[{"id":"WABA","changes":[{"value":{"messaging_product":"whatsapp","metadata":{"display_phone_number":"15550001234","phone_number_id":"'"$PNID"'"},"messages":[{"from":"5511999999999","id":"wamid.SMOKE","type":"text","text":{"body":"qual minha próxima tarefa?"}}]},"field":"messages"}]}]}' > "$DATA/payload.json"
SIG="$(python3 - "$SECRET" "$DATA/payload.json" <<'PY'
import hmac, hashlib, sys
secret = sys.argv[1].encode()
body = open(sys.argv[2], "rb").read()
print("sha256=" + hmac.new(secret, body, hashlib.sha256).hexdigest())
PY
)"

echo "== 1. signed webhook → fast 200 + forwards tenant id to the brain =="
CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 -X POST "$BASE/api/v1/whatsapp/webhook" \
  -H "Content-Type: application/json" -H "X-Hub-Signature-256: $SIG" --data-binary @"$DATA/payload.json")"
[ "$CODE" = "200" ] && ok "valid signature accepted (200, fast ack)" || bad "expected 200, got $CODE"
# co-web replies async — wait for the brain to capture the forwarded request.
for i in $(seq 1 20); do [ -s "$CAPTURE" ] && break; sleep 0.5; done
if [ -s "$CAPTURE" ]; then
  GOT_PNID="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("phone_number_id",""))' "$CAPTURE")"
  GOT_TEXT="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("text",""))' "$CAPTURE")"
  GOT_SENDER="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("sender",""))' "$CAPTURE")"
  [ "$GOT_PNID" = "$PNID" ] && ok "phone_number_id forwarded ($GOT_PNID)" || bad "tenant id mismatch: '$GOT_PNID'"
  [ "$GOT_TEXT" = "qual minha próxima tarefa?" ] && ok "message text forwarded intact (utf-8)" || bad "text mismatch: '$GOT_TEXT'"
  [ "$GOT_SENDER" = "whatsapp" ] && ok "sender tagged 'whatsapp'" || bad "sender: '$GOT_SENDER'"
else
  bad "brain never received the forwarded message (reply path did not run)"
fi

echo "== 2. forged signature → 401 (HMAC gate) =="
CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 -X POST "$BASE/api/v1/whatsapp/webhook" \
  -H "Content-Type: application/json" -H "X-Hub-Signature-256: sha256=deadbeef" --data-binary @"$DATA/payload.json")"
[ "$CODE" = "401" ] && ok "forged signature rejected (401)" || bad "expected 401, got $CODE"

echo "== 3. GET verification handshake =="
CHAL="challenge-$$"
BODY="$(curl -s --max-time 5 "$BASE/api/v1/whatsapp/webhook?hub.mode=subscribe&hub.verify_token=$VERIFY&hub.challenge=$CHAL")"
[ "$BODY" = "$CHAL" ] && ok "correct verify_token echoes hub.challenge" || bad "challenge not echoed: '$BODY'"
CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$BASE/api/v1/whatsapp/webhook?hub.mode=subscribe&hub.verify_token=wrong&hub.challenge=$CHAL")"
[ "$CODE" = "403" ] && ok "wrong verify_token rejected (403)" || bad "expected 403, got $CODE"

echo
echo "== RESULT: $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ] && { echo "🎉 Meta→co-web→brain path verified locally (no Meta)"; exit 0; } || { echo "see $DATA/server.log"; exit 1; }
