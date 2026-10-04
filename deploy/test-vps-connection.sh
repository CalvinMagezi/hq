#!/usr/bin/env bash
# VPS Agent MCP & Vault Connection Diagnostic Tool
# Usage: ./deploy/test-vps-connection.sh <https://your-domain.com> <AGENTHQ_API_KEY>

set -euo pipefail

DOMAIN="${1:-}"
API_KEY="${2:-}"

if [[ -z "$DOMAIN" || -z "$API_KEY" ]]; then
  echo "Usage: $0 <https://your-domain.com> <agent-api-key>"
  echo "Example: $0 https://mcp.your-domain.com \"$AGENTHQ_API_KEY\""
  exit 1
fi

echo "=================================================="
echo " HQ VPS Agent Connection Diagnostic"
echo " Target: $DOMAIN"
echo "=================================================="

# 1. Health Check
echo -n "[1/4] Checking HTTP /health endpoint... "
HEALTH_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$DOMAIN/health" || echo "000")
if [[ "$HEALTH_STATUS" == "200" ]]; then
  echo "OK (HTTP 200)"
else
  echo "FAILED (HTTP $HEALTH_STATUS)"
  echo "Error: Domain or Caddy proxy is not reaching HQ."
  exit 1
fi

# 2. MCP Handshake (initialize)
echo -n "[2/4] Testing MCP 'initialize' handshake... "
INIT_PAYLOAD='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"vps-test","version":"1.0"}}}'

INIT_RESP=$(curl -s -X POST "$DOMAIN/mcp" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "$INIT_PAYLOAD")

if echo "$INIT_RESP" | grep -q '"protocolVersion"'; then
  echo "OK"
else
  echo "FAILED"
  echo "Response: $INIT_RESP"
  exit 1
fi

# 3. Tools Listing (tools/list)
echo -n "[3/4] Testing MCP 'tools/list'... "
LIST_PAYLOAD='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'

LIST_RESP=$(curl -s -X POST "$DOMAIN/mcp" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "$LIST_PAYLOAD")

if echo "$LIST_RESP" | grep -q '"hq_discover"' && echo "$LIST_RESP" | grep -q '"hq_call"'; then
  echo "OK (Gateway tools hq_discover & hq_call verified)"
else
  echo "FAILED"
  echo "Response: $LIST_RESP"
  exit 1
fi

# 4. Discovery Tool Call (hq_discover)
echo -n "[4/4] Executing tool 'hq_discover' via hq_call... "
DISCOVER_PAYLOAD='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"hq_discover","arguments":{}}}'

DISCOVER_RESP=$(curl -s -X POST "$DOMAIN/mcp" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "$DISCOVER_PAYLOAD")

if echo "$DISCOVER_RESP" | grep -q '"result"'; then
  echo "OK (Vault discovery successful)"
else
  echo "FAILED"
  echo "Response: $DISCOVER_RESP"
  exit 1
fi

echo "=================================================="
echo " SUCCESS: VPS Agent connection to HQ is 100% operational!"
echo "=================================================="
