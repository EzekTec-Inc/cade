import { CadeStreamEvent } from "./events";

export interface SessionOptions {
  serverUrl?: string;
  apiKey?: string;
  agentId?: string;
  model?: string;
  systemPrompt?: string;
  cwd?: string;
}

export class AgentSession {
  private serverUrl: string;
  private apiKey: string;
  private agentId: string;

  constructor(options: SessionOptions = {}) {
    this.serverUrl = options.serverUrl || "http://localhost:8284";
    this.apiKey = options.apiKey || "";
    this.agentId = options.agentId || `node-agent-${Date.now()}`;
  }

  get id(): string {
    return this.agentId;
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

    const textOutput = await res.text();
    return textOutput;
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
