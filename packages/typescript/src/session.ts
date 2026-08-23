import { CadeStreamEvent } from "./events";

export interface SessionOptions {
  serverUrl?: string;
  apiKey?: string;
  agentId?: string;
  model?: string;
  systemPrompt?: string;
  cwd?: string;
}

export interface SubagentsClient {
  steer(subagentId: string, message: string): Promise<boolean>;
  cancel(subagentId: string): Promise<boolean>;
}

export class AgentSession {
  private serverUrl: string;
  private apiKey: string;
  private agentId: string;

  constructor(options: SessionOptions = {}) {
    this.serverUrl = (options.serverUrl || "http://localhost:8284").replace(/\/+$/, "");
    this.apiKey = options.apiKey || "";
    this.agentId = options.agentId || `node-agent-${Date.now()}`;
  }

  get id(): string {
    return this.agentId;
  }

  get subagents(): SubagentsClient {
    return {
      steer: (subagentId: string, message: string) => this.steerSubagent(subagentId, message),
      cancel: (subagentId: string) => this.cancelSubagent(subagentId),
    };
  }

  async prompt(text: string): Promise<string> {
    const res = await fetch(`${this.serverUrl}/v1/agents/${this.agentId}/run`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
      body: JSON.stringify({ input: text }),
    });

    if (!res.ok) {
      throw new Error(`CADE Server error: ${res.status} ${res.statusText}`);
    }

    return await res.text();
  }

  async *stream(text: string): AsyncGenerator<CadeStreamEvent, void, unknown> {
    const res = await fetch(`${this.serverUrl}/v1/agents/${this.agentId}/run/stream`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
      body: JSON.stringify({ input: text }),
    });

    if (!res.ok || !res.body) {
      throw new Error(`CADE Server stream error: ${res.status} ${res.statusText}`);
    }

    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });

      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        const trimmed = line.trim();
        if (trimmed.startsWith("data:")) {
          const rawJson = trimmed.slice(5).trim();
          if (!rawJson || rawJson === "[DONE]") continue;
          try {
            const event = JSON.parse(rawJson) as CadeStreamEvent;
            yield event;
          } catch {
            // Non-JSON or raw text fallback
            yield { type: "message_delta", data: rawJson };
          }
        }
      }
    }
  }

  async steerSubagent(subagentId: string, message: string): Promise<boolean> {
    const res = await fetch(`${this.serverUrl}/v1/subagents/${subagentId}/steer`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
      body: JSON.stringify({ message }),
    });
    return res.ok;
  }

  async cancelSubagent(subagentId: string): Promise<boolean> {
    const res = await fetch(`${this.serverUrl}/v1/subagents/${subagentId}/cancel`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
    });
    return res.ok;
  }

  async setMemory(label: string, value: string): Promise<void> {
    await fetch(`${this.serverUrl}/v1/agents/${this.agentId}/memory`, {
      method: "PUT",
      headers: {
        "Content-Type": "application/json",
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
      body: JSON.stringify({ label, value }),
    });
  }

  async getMemory(label: string): Promise<string | null> {
    const res = await fetch(`${this.serverUrl}/v1/agents/${this.agentId}/memory`, {
      headers: this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {},
    });
    if (!res.ok) return null;
    const data = (await res.json()) as Array<{ label: string; value: string }>;
    const found = data.find((b) => b.label === label);
    return found ? found.value : null;
  }
}
