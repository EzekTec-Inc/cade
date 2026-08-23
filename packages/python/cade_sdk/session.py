import ctypes
import os
from typing import Optional

def _load_lib():
    search_paths = [
        os.path.join(os.path.dirname(__file__), "../../../target/release/libcade_sdk.so"),
        os.path.join(os.path.dirname(__file__), "../../../target/debug/libcade_sdk.so"),
        os.path.join(os.path.dirname(__file__), "../../../target/release/libcade_sdk.dylib"),
        os.path.join(os.path.dirname(__file__), "../../../target/debug/libcade_sdk.dylib"),
        os.path.join(os.path.dirname(__file__), "../../../target/release/cade_sdk.dll"),
        os.path.join(os.path.dirname(__file__), "../../../target/debug/cade_sdk.dll"),
    ]
    for p in search_paths:
        if os.path.exists(p):
            try:
                lib = ctypes.CDLL(os.path.abspath(p))
                _setup_signatures(lib)
                return lib
            except Exception:
                continue
    return None

def _setup_signatures(lib):
    lib.cade_string_free.argtypes = [ctypes.c_char_p]
    lib.cade_string_free.restype = None

    lib.cade_embedded_session_create.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.cade_embedded_session_create.restype = ctypes.c_void_p

    lib.cade_embedded_session_prompt.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    lib.cade_embedded_session_prompt.restype = ctypes.c_void_p

    lib.cade_embedded_session_set_memory.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.cade_embedded_session_set_memory.restype = ctypes.c_int

    lib.cade_embedded_session_get_memory.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    lib.cade_embedded_session_get_memory.restype = ctypes.c_void_p

    lib.cade_embedded_session_free.argtypes = [ctypes.c_void_p]
    lib.cade_embedded_session_free.restype = None

    lib.cade_team_session_create.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_int]
    lib.cade_team_session_create.restype = ctypes.c_void_p

    lib.cade_team_session_run.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    lib.cade_team_session_run.restype = ctypes.c_void_p

    lib.cade_team_session_free.argtypes = [ctypes.c_void_p]
    lib.cade_team_session_free.restype = None

class EmbeddedSession:
    """In-process zero-daemon agent session linking directly to SQLite and LLM provider."""

    def __init__(self, db_path: Optional[str] = None, model: Optional[str] = None, system_prompt: Optional[str] = None):
        self._lib = _load_lib()
        if not self._lib:
            raise RuntimeError("Could not find libcade_sdk binary. Run `cargo build -p cade-sdk` first.")

        db_bytes = db_path.encode("utf-8") if db_path else None
        model_bytes = model.encode("utf-8") if model else None
        sys_bytes = system_prompt.encode("utf-8") if system_prompt else None

        self._handle = self._lib.cade_embedded_session_create(db_bytes, model_bytes, sys_bytes)
        if not self._handle:
            raise RuntimeError("Failed to initialize EmbeddedSession in CADE runtime.")

    def prompt(self, text: str) -> str:
        """Send a prompt and execute the agentic loop to convergence in-process."""
        if not self._handle:
            raise RuntimeError("Session has been closed.")
        res_ptr = self._lib.cade_embedded_session_prompt(self._handle, text.encode("utf-8"))
        if not res_ptr:
            return ""
        val = ctypes.cast(res_ptr, ctypes.c_char_p).value.decode("utf-8")
        self._lib.cade_string_free(ctypes.cast(res_ptr, ctypes.c_char_p))
        return val

    def set_memory(self, label: str, value: str) -> bool:
        """Set a persistent memory block."""
        if not self._handle:
            raise RuntimeError("Session has been closed.")
        rc = self._lib.cade_embedded_session_set_memory(
            self._handle, label.encode("utf-8"), value.encode("utf-8")
        )
        return rc == 0

    def get_memory(self, label: str) -> Optional[str]:
        """Retrieve the value of a memory block."""
        if not self._handle:
            raise RuntimeError("Session has been closed.")
        res_ptr = self._lib.cade_embedded_session_get_memory(self._handle, label.encode("utf-8"))
        if not res_ptr:
            return None
        val = ctypes.cast(res_ptr, ctypes.c_char_p).value.decode("utf-8")
        self._lib.cade_string_free(ctypes.cast(res_ptr, ctypes.c_char_p))
        return val

    def close(self):
        """Free session resources."""
        if self._handle and self._lib:
            self._lib.cade_embedded_session_free(self._handle)
            self._handle = None

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.close()

import json
import urllib.request
import urllib.error

class RemoteSession:
    """Remote client connecting over HTTP/SSE to cade-server."""

    def __init__(self, server_url: str = "http://localhost:8284", api_key: Optional[str] = None, agent_id: Optional[str] = None):
        self.server_url = server_url.rstrip("/")
        self.api_key = api_key or ""
        self.agent_id = agent_id or "default-agent"

    def prompt(self, text: str) -> str:
        req = urllib.request.Request(
            f"{self.server_url}/v1/agents/{self.agent_id}/run",
            data=json.dumps({"input": text}).encode("utf-8"),
            headers={"Content-Type": "application/json", **({"Authorization": f"Bearer {self.api_key}"} if self.api_key else {})},
            method="POST",
        )
        with urllib.request.urlopen(req) as resp:
            return resp.read().decode("utf-8")

    def steer_subagent(self, subagent_id: str, message: str) -> bool:
        req = urllib.request.Request(
            f"{self.server_url}/v1/subagents/{subagent_id}/steer",
            data=json.dumps({"message": message}).encode("utf-8"),
            headers={"Content-Type": "application/json", **({"Authorization": f"Bearer {self.api_key}"} if self.api_key else {})},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req) as resp:
                return resp.status == 200
        except urllib.error.URLError:
            return False

    def cancel_subagent(self, subagent_id: str) -> bool:
        req = urllib.request.Request(
            f"{self.server_url}/v1/subagents/{subagent_id}/cancel",
            headers={"Content-Type": "application/json", **({"Authorization": f"Bearer {self.api_key}"} if self.api_key else {})},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req) as resp:
                return resp.status == 200
        except urllib.error.URLError:
            return False
