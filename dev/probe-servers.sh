#!/bin/sh
# A one-off look at real MCP servers from the build box (not kept): each
# server's answer to initialize, its sign-in metadata, its tool list
# without a sign-in, and whether its provider accepts a sign-in request
# with Alpha Vector's client ID (catalog/registrations.toml).
set -u
init='{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"magnus","version":"0"}}}'
list='{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
H='-H content-type:application/json -H accept:application/json,text/event-stream -H mcp-protocol-version:2025-06-18'
probe() {
  u=$1
  echo "=================== $u"
  echo "--- initialize"; curl -sS -m 20 -D - -X POST $H -d "$init" "$u" | head -c 1500; echo
  o=$(echo "$u" | sed -E 's#^(https://[^/]+).*#\1#'); p=$(echo "$u" | sed -E 's#^https://[^/]+##; s#/$##')
  echo "--- protected resource metadata"; curl -sS -m 20 "$o/.well-known/oauth-protected-resource$p" | head -c 1500; echo
  echo "--- protected resource metadata (root)"; curl -sS -m 20 "$o/.well-known/oauth-protected-resource" | head -c 1500; echo
  echo "--- authorization server metadata (origin)"; curl -sS -m 20 "$o/.well-known/oauth-authorization-server" | head -c 2500; echo
  echo "--- tools/list without a sign-in"; curl -sS -m 20 -X POST $H -d "$list" "$u" | head -c 6000; echo
}
for u in https://workiq.svc.cloud.microsoft/mcp \
  "https://agent365.svc.cloud.microsoft/agents/tenants/f1524b07-679e-443c-838b-594fb9e8576a/servers/mcp_MailTools" \
  https://api.githubcopilot.com/mcp/ https://mcp.vercel.com https://mcp.slack.com/mcp https://mcp.sentry.dev/mcp \
  https://mcp.notion.com/mcp https://mcp.linear.app/mcp; do probe "$u"; done
echo "=================== GitHub authorization server"; curl -sS -m 20 https://github.com/.well-known/oauth-authorization-server | head -c 1500; echo
echo "=================== Slack openid"; curl -sS -m 20 https://slack.com/.well-known/openid-configuration | head -c 1500; echo
echo "=================== Vercel openid"; curl -sS -m 20 https://vercel.com/.well-known/openid-configuration | head -c 1500; echo
echo "=================== Sentry auth server"; curl -sS -m 20 https://sentry.io/.well-known/oauth-authorization-server | head -c 1500; echo
