#!/usr/bin/env python3
import sys
import json

def main():
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        
        try:
            req = json.loads(line.strip())
        except:
            continue
            
        if "id" not in req:
            continue
            
        req_id = req["id"]
        method = req.get("method")
        
        if method == "initialize":
            resp = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
                }
            }
        elif method == "tools/list":
            resp = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "tools": [{
                        "name": "show_test_ui",
                        "description": "Show a test UI",
                        "inputSchema": {"type": "object", "properties": {}}
                    }]
                }
            }
        elif method == "tools/call":
            resp = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": "Successfully triggered UI!"}],
                    "_meta": {
                        "ui": {
                            "resourceUri": "http://127.0.0.1:8080/ui.json"
                        }
                    }
                }
            }
        else:
            resp = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {}
            }
            
        print(json.dumps(resp), flush=True)

if __name__ == "__main__":
    main()