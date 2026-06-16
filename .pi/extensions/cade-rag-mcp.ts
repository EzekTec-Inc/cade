import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawn } from "child_process";

export default function cadeRagMcpExtension(pi: ExtensionAPI) {
    let child: any = null;
    let nextId = 1;
    const pendingRequests = new Map<number, { resolve: (val: any) => void; reject: (err: any) => void }>();
    let buffer = "";
    let didChangeCode = false;

    const startMcpServer = () => {
        child = spawn("/home/engr-uba/Downloads/02 Rust-project/mcp-servers/cade-rag-mcp/target/release/cade-rag-mcp");
        
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
                    } catch (err) { }
                }
            }
            buffer = lines[lines.length - 1];
        });
    };

    const sendRequest = (method: string, params: any): Promise<any> => {
        return new Promise((resolve, reject) => {
            const id = nextId++;
            pendingRequests.set(id, { resolve, reject });
            const req = { jsonrpc: "2.0", id, method, params };
            child.stdin.write(JSON.stringify(req) + "\n");
        });
    };

    const runWorkspaceIndex = async (ctx: any) => {
        try {
            const cwd = process.cwd();
            ctx.ui.setStatus("cade-rag", "Indexing workspace...");
            ctx.ui.notify(`Starting RAG indexing on: ${cwd}`, "info");
            
            await sendRequest("tools/call", {
                name: "index_workspace",
                arguments: { path: cwd }
            });
            
            ctx.ui.setStatus("cade-rag", "Workspace indexed");
            ctx.ui.notify("RAG workspace indexing complete!", "success");
        } catch (err: any) {
            ctx.ui.setStatus("cade-rag", "RAG Index Error");
            ctx.ui.notify(`RAG workspace indexing failed: ${err.message}`, "error");
        }
    };

    // Listen for code modification tools
    pi.on("tool_execution_end", (event) => {
        const mutatingTools = [
            "edit", "write",
            "serena_replace_content", "serena_replace_symbol_body",
            "serena_insert_after_symbol", "serena_insert_before_symbol",
            "serena_rename_symbol"
        ];
        if (mutatingTools.includes(event.toolName)) {
            didChangeCode = true;
        }
    });

    // Re-index at the end of a turn if changes were detected
    pi.on("turn_end", async (_event, ctx) => {
        if (didChangeCode) {
            didChangeCode = false;
            await runWorkspaceIndex(ctx);
        }
    });

    pi.on("session_start", async (_event, ctx) => {
        try {
            startMcpServer();
            await sendRequest("initialize", {
                protocolVersion: "2024-11-05",
                capabilities: {},
                clientInfo: { name: "pi", version: "1.0" }
            });

            const result = await sendRequest("tools/list", {});
            const tools = result.tools || [];

            ctx.ui.notify(`Found ${tools.length} RAG MCP tools. Registering...`, "info");

            for (const tool of tools) {
                pi.registerTool({
                    name: `rag_${tool.name}`,
                    label: tool.title || tool.name,
                    description: tool.description,
                    parameters: tool.inputSchema,
                    async execute(toolCallId, params) {
                        const res = await sendRequest("tools/call", { name: tool.name, arguments: params });
                        const content = (res.content || []).map((c: any) => ({
                            type: "text",
                            text: c.type === "text" ? c.text : JSON.stringify(c)
                        }));
                        return { content, isError: res.isError || false };
                    }
                });
            }

            ctx.ui.notify(`Successfully loaded RAG MCP tools!`, "success");
            
            // Automatically index workspace at start of session
            await runWorkspaceIndex(ctx);
            
        } catch (err: any) {
            ctx.ui.notify(`Failed to load RAG MCP: ${err.message}`, "error");
        }
    });
}
