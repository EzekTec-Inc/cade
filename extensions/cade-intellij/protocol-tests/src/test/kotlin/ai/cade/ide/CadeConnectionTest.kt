package ai.cade.ide

import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import java.io.*
import java.net.ServerSocket
import java.net.Socket
import java.nio.file.Paths
import kotlin.io.path.*
import kotlin.test.*

/** Minimal logging facade that captures log lines for assertions. */
private class CapturingLogger : CadeLogger {
    val lines = mutableListOf<String>()
    override fun info(msg: String) { lines += msg }
    override fun warn(msg: String) { lines += msg }
}

/** Blocking test TCP server that accepts one client. */
private class TestServer(port: Int = 0) : AutoCloseable {
    private val serverSocket = ServerSocket(port)
    val boundPort: Int get() = serverSocket.localPort

    fun acceptClient(): TestClient {
        val sock = serverSocket.accept()
        return TestClient(sock)
    }

    override fun close() { try { serverSocket.close() } catch (_: Exception) {} }
}

private class TestClient(private val sock: Socket) : AutoCloseable {
    private val reader = BufferedReader(InputStreamReader(sock.getInputStream(), Charsets.UTF_8))
    private val writer = PrintWriter(BufferedWriter(OutputStreamWriter(sock.getOutputStream(), Charsets.UTF_8)), true)

    fun readLine(): String = reader.readLine() ?: error("EOF")
    fun writeLine(line: String) = writer.println(line)
    fun writeServerMessage(msg: ServerMessage) = writeLine(
        cadeJson.encodeToString(ServerMessage.serializer(), msg).trimEnd('\n')
    )

    override fun close() { try { sock.close() } catch (_: Exception) {} }
}

/** Write a discovery file for the test server and return the file path. */
private fun writeDiscovery(port: Int): java.nio.file.Path {
    val dir = Paths.get(System.getProperty("user.home"), ".cade", "ide")
    dir.createDirectories()
    val path = dir.resolve("test-conn-${ProcessHandle.current().pid()}.json")
    path.writeText("""{"pid":${ProcessHandle.current().pid()},"addr":"127.0.0.1:$port"}""")
    return path
}

class CadeConnectionTest {

    // ── Hello frame ───────────────────────────────────────────────────────────

    @Test
    fun `sends Hello after connecting`() = runBlocking {
        TestServer().use { server ->
            val disc = writeDiscovery(server.boundPort)
            try {
                val log = CapturingLogger()
                val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
                val conn = CadeConnection(scope, log)

                val clientJob = launch(Dispatchers.IO) { conn.connect() }
                val client = server.acceptClient()
                val line = client.readLine()
                val msg = decodeAdapterMessage(line)
                assertTrue(msg is AdapterMessage.Hello)
                assertEquals(1, (msg as AdapterMessage.Hello).protocol_version)

                // Send HelloAck.
                client.writeServerMessage(ServerMessage.HelloAck(1))
                delay(30)
                assertTrue(log.lines.any { it.contains("HelloAck") })

                clientJob.cancel()
                conn.dispose()
                scope.cancel()
                client.close()
            } finally { disc.deleteIfExists() }
        }
    }

    // ── sendStateUpdate ───────────────────────────────────────────────────────

    @Test
    fun `sendStateUpdate writes a state_update frame`() = runBlocking {
        TestServer().use { server ->
            val disc = writeDiscovery(server.boundPort)
            try {
                val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
                val conn = CadeConnection(scope)

                launch(Dispatchers.IO) { conn.connect() }
                val client = server.acceptClient()
                client.readLine() // Hello
                client.writeServerMessage(ServerMessage.HelloAck(1))
                delay(30)

                conn.sendStateUpdate(StateSnapshot(
                    open_files = emptyList(), active_file = "/tmp/a.kt",
                    selection = null, diagnostics = emptyList(),
                    workspace_folders = emptyList(), visible_range = null,
                ))

                val line = client.readLine()
                val msg = decodeAdapterMessage(line)
                assertTrue(msg is AdapterMessage.StateUpdate)
                assertEquals("/tmp/a.kt", (msg as AdapterMessage.StateUpdate).active_file)

                conn.dispose(); scope.cancel(); client.close()
            } finally { disc.deleteIfExists() }
        }
    }

    // ── sendResponse ──────────────────────────────────────────────────────────

    @Test
    fun `sendResponse writes a callback_response frame`() = runBlocking {
        TestServer().use { server ->
            val disc = writeDiscovery(server.boundPort)
            try {
                val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
                val conn = CadeConnection(scope)

                launch(Dispatchers.IO) { conn.connect() }
                val client = server.acceptClient()
                client.readLine()
                client.writeServerMessage(ServerMessage.HelloAck(1))
                delay(30)

                conn.sendResponse(77L, CallbackResult.Ok)
                val line = client.readLine()
                val msg = decodeAdapterMessage(line)
                assertTrue(msg is AdapterMessage.CallbackResponse)
                assertEquals(77L, (msg as AdapterMessage.CallbackResponse).id)
                assertEquals(CallbackResult.Ok, msg.result)

                conn.dispose(); scope.cancel(); client.close()
            } finally { disc.deleteIfExists() }
        }
    }

    // ── CallbackRequest dispatch ──────────────────────────────────────────────

    @Test
    fun `dispatches CallbackRequest to handler`() = runBlocking {
        TestServer().use { server ->
            val disc = writeDiscovery(server.boundPort)
            try {
                val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
                val conn = CadeConnection(scope)
                val received = mutableListOf<Pair<Long, CallbackOp>>()
                conn.onCallbackRequest { id, op -> received += id to op }

                launch(Dispatchers.IO) { conn.connect() }
                val client = server.acceptClient()
                client.readLine()
                client.writeServerMessage(ServerMessage.HelloAck(1))
                delay(30)

                client.writeServerMessage(ServerMessage.CallbackRequest(
                    id = 55L, op = CallbackOp.RunTask("Build"),
                ))
                delay(50)

                assertEquals(1, received.size)
                assertEquals(55L, received[0].first)
                assertEquals(CallbackOp.RunTask("Build"), received[0].second)

                conn.dispose(); scope.cancel(); client.close()
            } finally { disc.deleteIfExists() }
        }
    }

    // ── No discovery file ─────────────────────────────────────────────────────

    @Test
    fun `no discovery file logs warning without throwing`() = runBlocking {
        val missingDir = Paths.get(System.getProperty("user.home"), ".cade", "ide")
        // Delete any test discovery files.
        if (missingDir.exists()) {
            missingDir.listDirectoryEntries("test-conn-*.json").forEach { it.deleteIfExists() }
        }

        val log = CapturingLogger()
        val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
        val conn = CadeConnection(scope, log)
        conn.connect()
        delay(50)

        assertTrue(log.lines.isNotEmpty(), "expected a warning log line")
        conn.dispose(); scope.cancel()
    }

    // ── dispose ───────────────────────────────────────────────────────────────

    @Test
    fun `dispose prevents further connections`() = runBlocking {
        val log = CapturingLogger()
        val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
        val conn = CadeConnection(scope, log)
        conn.dispose()
        conn.connect()
        delay(30)
        assertTrue(log.lines.isEmpty(), "disposed connection should not log anything")
        scope.cancel()
    }
}

/** Decode a newline-terminated adapter message line. */
private fun decodeAdapterMessage(line: String): AdapterMessage =
    cadeJson.decodeFromString(line.trim())
