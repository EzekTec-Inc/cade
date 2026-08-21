export type CadeStreamEventType =
  | "thought"
  | "message_delta"
  | "tool_executing"
  | "tool_completed"
  | "approval_required"
  | "usage"
  | "finished"
  | "error";

export interface ToolExecutingData {
  tool_call_id: string;
  tool_name: string;
  arguments: Record<string, unknown>;
}

export interface ToolCompletedData {
  tool_call_id: string;
  tool_name: string;
  output: string;
  is_error: boolean;
}

export interface UsageData {
  input_tokens: number;
  output_tokens: number;
  model: string;
}

export interface CadeStreamEvent {
  type: CadeStreamEventType;
  data: string | ToolExecutingData | ToolCompletedData | UsageData | Record<string, unknown>;
}
