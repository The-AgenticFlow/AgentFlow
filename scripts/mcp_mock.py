#!/usr/bin/env python3
import json
import sys

class MCPMockServer:
    def __init__(self, name, version="1.0.0", responses=None, tools=None):
        self.name = name
        self.version = version
        self.responses = responses or {}
        self.tools = tools or []

    def handle_request(self, request):
        method = request.get("method", "")
        params = request.get("params", {})
        
        if method == "initialize":
            return {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": self.name, "version": self.version}
            }
        elif method == "tools/list":
            return {"tools": self.tools}
        elif method == "tools/call":
            tool_name = params.get("name", "")
            if tool_name in self.responses:
                return {"content": [{"type": "text", "text": json.dumps(self.responses[tool_name])}]}
            return {"error": f"Unknown tool: {tool_name}"}
        return {"error": f"Unknown method: {method}"}

    def run(self):
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                request = json.loads(line)
                response = {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "result": self.handle_request(request)
                }
                print(json.dumps(response), flush=True)
            except Exception as e:
                print(json.dumps({
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32603, "message": str(e)}
                }), flush=True)

if __name__ == "__main__":
    # Example usage:
    # server = MCPMockServer("example", responses={"hello": "world"})
    # server.run()
    pass
