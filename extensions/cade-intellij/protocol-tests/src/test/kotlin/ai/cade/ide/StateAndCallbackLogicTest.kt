package ai.cade.ide

import kotlin.test.*

/**
 * Unit tests for pure logic extracted from StatePublisher and CallbackHandler
 * that does not require the IntelliJ Platform.
 */
class StateAndCallbackLogicTest {

    // ── offsetToPosition (StatePublisher helper) ───────────────────────────────

    private fun offsetToPosition(text: String, offset: Int): CadePosition {
        var line = 0; var col = 0
        for (i in 0 until offset.coerceAtMost(text.length)) {
            if (text[i] == '\n') { line++; col = 0 } else col++
        }
        return CadePosition(line, col)
    }

    @Test
    fun `offsetToPosition beginning of file`() {
        assertEquals(CadePosition(0, 0), offsetToPosition("hello\nworld", 0))
    }

    @Test
    fun `offsetToPosition mid first line`() {
        assertEquals(CadePosition(0, 3), offsetToPosition("hello\nworld", 3))
    }

    @Test
    fun `offsetToPosition newline character itself`() {
        // offset 5 is the '\n'; moving past it increments line.
        assertEquals(CadePosition(0, 5), offsetToPosition("hello\nworld", 5))
    }

    @Test
    fun `offsetToPosition start of second line`() {
        assertEquals(CadePosition(1, 0), offsetToPosition("hello\nworld", 6))
    }

    @Test
    fun `offsetToPosition mid second line`() {
        assertEquals(CadePosition(1, 3), offsetToPosition("hello\nworld", 9))
    }

    @Test
    fun `offsetToPosition beyond end clamps to text length`() {
        val text = "hi"
        val pos = offsetToPosition(text, 100)
        assertEquals(CadePosition(0, 2), pos)
    }

    // ── positionToOffset (CallbackHandler helper) ──────────────────────────────

    private fun positionToOffset(text: String, pos: CadePosition): Int {
        var line = 0; var idx = 0
        while (idx < text.length && line < pos.line) {
            if (text[idx++] == '\n') line++
        }
        return (idx + pos.character).coerceAtMost(text.length)
    }

    @Test
    fun `positionToOffset line 0 char 0`() {
        assertEquals(0, positionToOffset("hello\nworld", CadePosition(0, 0)))
    }

    @Test
    fun `positionToOffset line 0 char 3`() {
        assertEquals(3, positionToOffset("hello\nworld", CadePosition(0, 3)))
    }

    @Test
    fun `positionToOffset line 1 char 0`() {
        assertEquals(6, positionToOffset("hello\nworld", CadePosition(1, 0)))
    }

    @Test
    fun `positionToOffset line 1 char 3`() {
        assertEquals(9, positionToOffset("hello\nworld", CadePosition(1, 3)))
    }

    @Test
    fun `positionToOffset clamps beyond text length`() {
        assertEquals(2, positionToOffset("hi", CadePosition(0, 100)))
    }

    @Test
    fun `positionToOffset and offsetToPosition are inverses for valid positions`() {
        val text = "fn main() {\n    println!(\"hello\");\n}\n"
        for (offset in listOf(0, 5, 11, 12, 17, 35)) {
            val pos = offsetToPosition(text, offset)
            val back = positionToOffset(text, pos)
            assertEquals(offset, back, "round-trip failed at offset $offset")
        }
    }

    // ── CallbackResult serialisation ──────────────────────────────────────────

    @Test
    fun `CallbackResult Ok serialises with status ok`() {
        val json = cadeJson.encodeToString(CallbackResult.serializer(), CallbackResult.Ok)
        assertTrue(json.contains("\"status\":\"ok\""), "json=$json")
    }

    @Test
    fun `CallbackResult Err serialises with status err and message`() {
        val json = cadeJson.encodeToString(CallbackResult.serializer(), CallbackResult.Err("oops"))
        assertTrue(json.contains("\"status\":\"err\""), "json=$json")
        assertTrue(json.contains("\"message\":\"oops\""), "json=$json")
    }

    @Test
    fun `CallbackResult Ok round-trips`() {
        val encoded = cadeJson.encodeToString(CallbackResult.serializer(), CallbackResult.Ok)
        val decoded = cadeJson.decodeFromString(CallbackResult.serializer(), encoded)
        assertEquals(CallbackResult.Ok, decoded)
    }

    @Test
    fun `CallbackResult Err round-trips`() {
        val msg = CallbackResult.Err("file not found")
        val encoded = cadeJson.encodeToString(CallbackResult.serializer(), msg)
        val decoded = cadeJson.decodeFromString(CallbackResult.serializer(), encoded)
        assertEquals(msg, decoded)
    }
}
