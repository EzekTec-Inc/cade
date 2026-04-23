package ai.cade.ide

import kotlin.test.*

class CadeProtocolTest {

    // ── Helper: round-trip an AdapterMessage through JSON ─────────────────────

    private fun roundTrip(msg: AdapterMessage): AdapterMessage {
        val line = msg.encode()
        // Must be one line (trailing \n, no embedded newlines).
        val body = line.trimEnd('\n')
        assertFalse(body.contains('\n'), "encoded message must not contain embedded newlines")
        return cadeJson.decodeFromString(body)
    }

    // ── AdapterMessage.Hello ──────────────────────────────────────────────────

    @Test
    fun `Hello round-trips`() {
        val msg = AdapterMessage.Hello(label = "intellij-2024.1", protocol_version = 1)
        assertEquals(msg, roundTrip(msg))
    }

    @Test
    fun `Hello serialises type tag`() {
        val json = AdapterMessage.Hello("test", 1).encode()
        assertTrue(json.contains("\"type\":\"hello\""), "json=$json")
        assertTrue(json.contains("\"label\":\"test\""), "json=$json")
        assertTrue(json.contains("\"protocol_version\":1"), "json=$json")
    }

    // ── AdapterMessage.StateUpdate ────────────────────────────────────────────

    @Test
    fun `StateUpdate empty snapshot round-trips`() {
        val msg = AdapterMessage.StateUpdate(
            open_files = emptyList(),
            active_file = null,
            selection = null,
            diagnostics = emptyList(),
            workspace_folders = emptyList(),
            visible_range = null,
        )
        assertEquals(msg, roundTrip(msg))
    }

    @Test
    fun `StateUpdate full snapshot round-trips`() {
        val msg = AdapterMessage.StateUpdate(
            open_files = listOf(
                CadeOpenFile("/tmp/a.kt", "fun main() {}", "kotlin", 3, true)
            ),
            active_file = "/tmp/a.kt",
            selection = CadeSelection(
                path = "/tmp/a.kt",
                range = CadeRange(CadePosition(0, 0), CadePosition(0, 3)),
                text = "fun",
            ),
            diagnostics = listOf(
                CadeDiagnostic(
                    path = "/tmp/a.kt",
                    range = CadeRange(CadePosition(0, 0), CadePosition(0, 3)),
                    severity = "error",
                    message = "unresolved reference",
                    source = "kotlin",
                    code = "UNRESOLVED_REFERENCE",
                )
            ),
            workspace_folders = listOf(CadeWorkspaceFolder("/tmp", "tmp")),
            visible_range = listOf(0, 40),
        )
        assertEquals(msg, roundTrip(msg))
    }

    // ── AdapterMessage.CallbackResponse ───────────────────────────────────────

    @Test
    fun `CallbackResponse Ok round-trips`() {
        val msg = AdapterMessage.CallbackResponse(id = 42L, result = CallbackResult.Ok)
        assertEquals(msg, roundTrip(msg))
    }

    @Test
    fun `CallbackResponse Err round-trips`() {
        val msg = AdapterMessage.CallbackResponse(
            id = 7L,
            result = CallbackResult.Err("file not open"),
        )
        assertEquals(msg, roundTrip(msg))
    }

    // ── ServerMessage.HelloAck ────────────────────────────────────────────────

    @Test
    fun `HelloAck deserialises correctly`() {
        val json = """{"type":"hello_ack","protocol_version":1}"""
        val msg = decodeServerMessage(json)
        assertEquals(ServerMessage.HelloAck(1), msg)
    }

    // ── ServerMessage.CallbackRequest / CallbackOp ────────────────────────────

    @Test
    fun `CallbackRequest ApplyEdit deserialises correctly`() {
        val json = """{"type":"callback_request","id":1,"op":{"op":"apply_edit","path":"/tmp/a.kt","text_edits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"new_text":"// hi\n"}]}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        assertEquals(1L, msg.id)
        val op = msg.op as CallbackOp.ApplyEdit
        assertEquals("/tmp/a.kt", op.path)
        assertEquals(1, op.text_edits.size)
        assertEquals("// hi\n", op.text_edits[0].new_text)
    }

    @Test
    fun `CallbackRequest RevealFile deserialises correctly`() {
        val json = """{"type":"callback_request","id":2,"op":{"op":"reveal_file","path":"/tmp/b.kt"}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        assertEquals(CallbackOp.RevealFile("/tmp/b.kt"), msg.op)
    }

    @Test
    fun `CallbackRequest SetSelection deserialises correctly`() {
        val json = """{"type":"callback_request","id":3,"op":{"op":"set_selection","path":"/tmp/c.kt","range":{"start":{"line":5,"character":2},"end":{"line":5,"character":10}}}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        val op = msg.op as CallbackOp.SetSelection
        assertEquals("/tmp/c.kt", op.path)
        assertEquals(5, op.range.start.line)
    }

    @Test
    fun `CallbackRequest Save single deserialises correctly`() {
        val json = """{"type":"callback_request","id":4,"op":{"op":"save","path":"/tmp/d.kt"}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        assertEquals(CallbackOp.Save("/tmp/d.kt"), msg.op)
    }

    @Test
    fun `CallbackRequest Save all deserialises correctly`() {
        val json = """{"type":"callback_request","id":5,"op":{"op":"save","path":null}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        assertEquals(CallbackOp.Save(null), msg.op)
    }

    @Test
    fun `CallbackRequest RunTask deserialises correctly`() {
        val json = """{"type":"callback_request","id":6,"op":{"op":"run_task","name":"Build"}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        assertEquals(CallbackOp.RunTask("Build"), msg.op)
    }

    @Test
    fun `CallbackRequest RunTerminal deserialises correctly`() {
        val json = """{"type":"callback_request","id":7,"op":{"op":"run_terminal","command":"cargo test"}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        assertEquals(CallbackOp.RunTerminal("cargo test"), msg.op)
    }

    @Test
    fun `CallbackRequest DebugControl start deserialises correctly`() {
        val json = """{"type":"callback_request","id":8,"op":{"op":"debug_control","action":"start","config":"unit-tests"}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        val op = msg.op as CallbackOp.DebugControl
        assertEquals("start", op.action)
        assertEquals("unit-tests", op.config)
    }

    @Test
    fun `CallbackRequest DebugControl stop deserialises correctly`() {
        val json = """{"type":"callback_request","id":9,"op":{"op":"debug_control","action":"stop"}}"""
        val msg = decodeServerMessage(json) as ServerMessage.CallbackRequest
        val op = msg.op as CallbackOp.DebugControl
        assertEquals("stop", op.action)
        assertNull(op.config)
    }

    @Test
    fun `encode produces no embedded newlines`() {
        val msg = AdapterMessage.Hello("test", 1)
        val line = msg.encode()
        assertTrue(line.endsWith("\n"))
        assertEquals(1, line.count { it == '\n' })
    }
}
