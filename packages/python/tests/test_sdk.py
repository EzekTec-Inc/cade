import unittest
import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))
from cade_sdk import EmbeddedSession, TeamSession, CadeStreamEvent

class TestCadeSdk(unittest.TestCase):
    def test_event_parsing(self):
        evt = CadeStreamEvent.from_dict({"type": "thought", "data": "Thinking about tests"})
        self.assertTrue(evt.is_thought())
        self.assertEqual(evt.data, "Thinking about tests")

    def test_embedded_session_memory(self):
        try:
            session = EmbeddedSession()
        except RuntimeError:
            self.skipTest("libcade_sdk not built yet")

        session.set_memory("test_key", "test_val")
        val = session.get_memory("test_key")
        self.assertEqual(val, "test_val")
        session.close()

if __name__ == "__main__":
    unittest.main()
