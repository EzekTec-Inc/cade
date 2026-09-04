export type TeamMode = "coordinate" | "route" | "tasks";

export interface TeamMemberConfig {
  id: string;
  name: string;
  role: string;
  systemPrompt?: string;
  tools?: string[] | "readonly" | "all";
}

export interface TeamSessionConfig {
  teamId?: string;
  name?: string;
  mode?: TeamMode;
  members?: TeamMemberConfig[];
  serverUrl?: string;
  apiKey?: string;
}

export interface TeamResultItem {
  taskIndex: number;
  output: string;
  isError: boolean;
}

export class TeamSession {
  private teamId: string;
  private name: string;
  private mode: TeamMode;
  private members: TeamMemberConfig[];

  constructor(config: TeamSessionConfig = {}) {
    this.teamId = config.teamId || `team-${Date.now()}`;
    this.name = config.name || "Collaborative Squad";
    this.mode = config.mode || "coordinate";
    this.members = config.members || [];
  }

  get id(): string {
    return this.teamId;
  }

  addMember(member: TeamMemberConfig): this {
    this.members.push(member);
    return this;
  }
}
