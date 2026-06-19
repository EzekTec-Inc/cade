import * as net from "net";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

export class BridgeConnection {
  private socket: net.Socket | null = null;
  private buffer: string = "";
  private isDisposedState: boolean = false;

  constructor(
    private readonly onMessage: (msg: any) => void,
    private readonly onConnected: () => void
  ) {}

  public async connect(): Promise<void> {
    if (this.isDisposedState) {
      return;
    }

    try {
      const port = await this.discoverPort();
      this.socket = net.createConnection({ port, host: "127.0.0.1" }, () => {
        this.send({ type: "hello", label: "vscode", protocol_version: 1 });
        this.onConnected();
      });

      this.socket.on("data", (data) => {
        this.buffer += data.toString();
        const lines = this.buffer.split("\n");
        this.buffer = lines.pop() || "";
        for (const line of lines) {
          if (line.trim()) {
            try {
              this.onMessage(JSON.parse(line));
            } catch {
              // Parse error ignored
            }
          }
        }
      });

      this.socket.on("close", () => this.retryConnection());
      this.socket.on("error", () => {
        // Quiet drop to avoid spamming the console
      });
    } catch {
      this.retryConnection();
    }
  }

  public send(msg: any): void {
    if (this.socket && !this.socket.destroyed) {
      this.socket.write(JSON.stringify(msg) + "\n");
    }
  }

  private async discoverPort(): Promise<number> {
    const ideDir = path.join(os.homedir(), ".cade", "ide");
    if (!fs.existsSync(ideDir)) {
      throw new Error("CADE ide directory does not exist");
    }
    const files = await fs.promises.readdir(ideDir);
    const jsonFiles = files.filter((f) => f.endsWith(".json"));
    if (jsonFiles.length === 0) {
      throw new Error("No CADE bridge active");
    }
    // Read the newest active server configuration
    const newestFile = jsonFiles[jsonFiles.length - 1];
    const raw = await fs.promises.readFile(path.join(ideDir, newestFile), "utf-8");
    return JSON.parse(raw).port;
  }

  private retryConnection(): void {
    if (this.isDisposedState) {
      return;
    }
    setTimeout(() => {
      if (!this.isDisposedState) {
        this.connect_after_delay();
      }
    }, 5000);
  }

  private async connect_after_delay(): Promise<void> {
    await this.connect();
  }

  public dispose(): void {
    this.isDisposedState = true;
    if (this.socket) {
      this.socket.destroy();
    }
  }
}
