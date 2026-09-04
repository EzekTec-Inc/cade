from dataclasses import dataclass
from typing import Optional, Dict, Any

@dataclass
class CadeStreamEvent:
    event_type: str
    data: Any

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "CadeStreamEvent":
        t = d.get("type", "unknown")
        data = d.get("data")
        return cls(event_type=t, data=data)

    def is_thought(self) -> bool:
        return self.event_type == "thought"

    def is_delta(self) -> bool:
        return self.event_type == "message_delta"

    def is_tool_executing(self) -> bool:
        return self.event_type == "tool_executing"

    def is_finished(self) -> bool:
        return self.event_type == "finished"
