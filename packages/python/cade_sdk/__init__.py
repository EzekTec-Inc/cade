"""CADE Python SDK — In-Process Agent Runtime, Team Squads, and Typed Telemetry."""

from .session import EmbeddedSession, RemoteSession
from .team import TeamSession
from .events import CadeStreamEvent

__all__ = ["EmbeddedSession", "RemoteSession", "TeamSession", "CadeStreamEvent"]
__version__ = "0.2.4"
