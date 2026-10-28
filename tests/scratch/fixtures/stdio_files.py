"""A stdio MCP server over one folder (argv[1]), for the connector tests.

It writes a notification before every answer, as servers may, and offers
`whoami` (its process id and environment) so a test can see it was kept
running and started with a cleared environment.
"""
import json
import os
import sys

ROOT = os.path.realpath(sys.argv[1])

TOOLS = [
    {"name": "list_directory", "description": "List a folder.", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
    {"name": "read_file", "description": "Read a file.", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
    {"name": "write_file", "description": "Write a file.", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]}},
    {"name": "whoami", "description": "Show the server process.", "inputSchema": {"type": "object", "properties": {}}},
]


def inside(p):
    full = os.path.realpath(os.path.join(ROOT, p.lstrip("/")))
    if not (full == ROOT or full.startswith(ROOT + os.sep)):
        raise ValueError("outside the shared folder")
    return full


def call(name, args):
    if name == "list_directory":
        return "\n".join(sorted(os.listdir(inside(args["path"]))))
    if name == "read_file":
        with open(inside(args["path"])) as f:
            return f.read()
    if name == "write_file":
        with open(inside(args["path"]), "w") as f:
            f.write(args["content"])
        return "written"
    if name == "whoami":
        return json.dumps({"pid": os.getpid(), "env": sorted(os.environ)})
    raise ValueError(f"no tool {name}")


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    try:
        req = json.loads(line)
    except ValueError:
        continue
    if "id" not in req:
        continue
    send({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "working"}})
    method, rid = req.get("method"), req["id"]
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "files", "version": "1"}}})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}})
    elif method == "tools/call":
        try:
            text = call(req["params"]["name"], req["params"].get("arguments") or {})
            send({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}]}})
        except Exception as e:
            send({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": str(e)}], "isError": True}})
    else:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "unknown method"}})
