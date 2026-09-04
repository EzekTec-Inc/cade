import ctypes
import json
from typing import Optional, List, Dict, Any
from .session import _load_lib

class TeamSession:
    """Multi-agent collaborative squad session."""

    def __init__(self, team_id: Optional[str] = None, name: Optional[str] = None, mode: str = "coordinate"):
        self._lib = _load_lib()
        if not self._lib:
            raise RuntimeError("Could not find libcade_sdk binary. Run `cargo build -p cade-sdk` first.")

        mode_id = 0
        if mode.lower() == "route":
            mode_id = 1
        elif mode.lower() == "tasks":
            mode_id = 2

        tid_bytes = team_id.encode("utf-8") if team_id else None
        name_bytes = name.encode("utf-8") if name else None

        self._handle = self._lib.cade_team_session_create(tid_bytes, name_bytes, mode_id)
        if not self._handle:
            raise RuntimeError("Failed to initialize TeamSession in CADE runtime.")

    def run(self, prompt: str) -> List[Dict[str, Any]]:
        """Dispatch a mission across the squad members synchronously."""
        if not self._handle:
            raise RuntimeError("Team session has been closed.")
        res_ptr = self._lib.cade_team_session_run(self._handle, prompt.encode("utf-8"))
        if not res_ptr:
            return []
        raw_json = ctypes.cast(res_ptr, ctypes.c_char_p).value.decode("utf-8")
        self._lib.cade_string_free(ctypes.cast(res_ptr, ctypes.c_char_p))
        try:
            return json.loads(raw_json)
        except Exception:
            return [{"output": raw_json, "is_error": True, "task_index": 0}]

    def close(self):
        """Free team resources."""
        if self._handle and self._lib:
            self._lib.cade_team_session_free(self._handle)
            self._handle = None

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.close()
