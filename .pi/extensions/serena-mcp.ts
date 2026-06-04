import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawn } from "child_process";

export default function serenaMcpExtension(pi: ExtensionAPI) {
    let child: any = null;
    let nextId = 1;
    const pendingRequests = new Map<number, { resolve: (val: any) => void; reject: (err: any) => void }>();
    let buffer = "";

    const startMcpServer = () => {
        child = spawn("/home/engr-uba/.local/bin/serena", ["start-mcp-server", "--context=claude-code", "--project-from-cwd"]);
        
        child.stdout.on("data", (data: Buffer) => {
            buffer += data.toString();
            const lines = buffer.split("\n");
            for (let i = 0; i < lines.length - 1; i++) {
                const line = lines[i].trim();
                if (line) {
                    try {
                        const msg = JSON.parse(line);
                        if (msg.id !== undefined && pendingRequests.has(msg.id)) {
                            const { resolve, reject } = pendingRequests.get(msg.id)!;
                            pendingRequests.delete(msg.id);
                            if (msg.error) {
                                reject(msg.error);
                            } else {
                                resolve(msg.result);
                            }
                        }
                    } catch (err) {
                        // Ignore parse errors for partial lines
                    }
                }
            }
            buffer = lines[lines.length - 1];
        });

        child.stderr.on("data", (data: Buffer) => {
            // Log server stderr if needed
        });
    };

    const sendRequest = (method: string, params: any): Promise<any> => {
        return new Promise((resolve, reject) => {
            const id = nextId++;
            pendingRequests.set(id, { resolve, reject });
            const req = {
                jsonrpc: "2.0",
                id,
                method,
                params
            };
            child.stdin.write(JSON.stringify(req) + "\n");
        });
    };

    pi.on("session_start", async (_event, ctx) => {
        try {
            startMcpServer();
            
            // 1. Initialize
            await sendRequest("initialize", {
                protocolVersion: "2024-11-05",
                capabilities: {},
                clientInfo: { name: "pi", version: "1.0" }
            });

            // 2. List tools
            const result = await sendRequest("tools/list", {});
            const tools = result.tools || [];

            ctx.ui.notify(`Found ${tools.length} Serena MCP tools. Registering...`, "info");

            // 3. Register each tool in pi
            for (const tool of tools) {
                // Prefix the name with serena_ to prevent name collisions
                const piToolName = `serena_${tool.name}`;
                
                pi.registerTool({
                    name: piToolName,
                    label: tool.title || tool.name,
                    description: tool.description,
                    parameters: tool.inputSchema,
                    async execute(toolCallId, params, signal, onUpdate, ctx) {
                        try {
                            const res = await sendRequest("tools/call", {
                                name: tool.name,
                                arguments: params
                            });
                            
                            // Convert standard MCP content back to Pi content format
                            const content = (res.content || []).map((c: any) => {
                                if (c.type === "text") {
                                    return { type: "text", text: c.text };
                                }
                                return { type: "text", text: JSON.stringify(c) };
                            });

                            return {
                                content,
                                isError: res.isError || false
                            };
                        } catch (err: any) {
                            return {
                                content: [{ type: "text", text: `Error calling Serena: ${err.message || JSON.stringify(err)}` }],
                                isError: true
                            };
                        }
                    }
                });
            }

            ctx.ui.notify(`Successfully loaded all ${tools.length} Serena MCP tools!`, "success");
        } catch (err: any) {
            ctx.ui.notify(`Failed to load Serena MCP: ${err.message}`, "error");
        }
    });
}
