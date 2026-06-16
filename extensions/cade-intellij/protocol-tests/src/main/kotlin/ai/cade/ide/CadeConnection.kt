@file:OptIn(ExperimentalSerializationApi::class)

package ai.cade.ide

import kotlinx.coroutines.*
import kotlinx.serialization.ExperimentalSerializationApi
import java.io.*
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.file.Path
import java.nio.file.Paths
import kotlin.io.path.*

private const val PROTOCOL_VERSION = 1
private const val RECONNECT_DELAY_MS = 3_000L

typealias CallbackRequestHandler = suspend (id: Long, op: CallbackOp) -> Unit

/** Minimal logging facade — injected so tests and the plugin can both use it. */
interface CadeLogger {
    fun info(msg: String)
    fun warn(msg: String)
}

/**
 * CadeConnection — manages the TCP connection to `cade-ide-mcp`.
 *
 * All IntelliJ-specific logging is done through [CadeLogger] so the
 * class can be instantiated in plain JVM tests without the platform.
 */
class CadeConnection(
    private val scope: CoroutineScope,
    private val log: CadeLogger = NoOpLogger,
) {
    private var socket: Socket? = null
    private var writer: PrintWriter? = null
    private var readJob: Job? = null
    private var handler: CallbackRequestHandler? = null
    @Volatile private var disposed = false

    fun onCallbackRequest(h: CallbackRequestHandler) { handler = h }

    /** Connect to `cade-ide-mcp` using the discovery file. */
    suspend fun connect() {
        if (disposed) return
        disconnect()

        val info = readDiscoveryFile()
        if (info == null) {
            log.warn("CADE: cade-ide-mcp not running (no discovery file). Retrying…")
            scheduleReconnect()
            return
        }

        val parts = info.addr.split(":")
        val host = parts[0]
        val port = parts.getOrNull(1)?.toIntOrNull() ?: run {
            log.warn("CADE: invalid addr '${info.addr}'")
            scheduleReconnect()
            return
        }

        try {
            val sock = Socket()
            sock.connect(InetSocketAddress(host, port), 5_000)
            socket = sock
            val pw = PrintWriter(
                BufferedWriter(OutputStreamWriter(sock.getOutputStream(), Charsets.UTF_8)),
                true,
            )
            writer = pw
            pw.println(AdapterMessage.Hello("intellij-cade", PROTOCOL_VERSION).encodeNoNewline())
            log.info("CADE: connected to cade-ide-mcp at ${info.addr}")
            readJob = scope.launch(Dispatchers.IO) { readLoop(sock) }
        } catch (e: Exception) {
            log.warn("CADE: connection failed — ${e.message}")
            scheduleReconnect()
        }
    }

    fun sendStateUpdate(snap: StateSnapshot) {
        writeLine(AdapterMessage.StateUpdate(
            open_files = snap.open_files,
            active_file = snap.active_file,
            selection = snap.selection,
            diagnostics = snap.diagnostics,
            workspace_folders = snap.workspace_folders,
            visible_range = snap.visible_range,
        ).encodeNoNewline())
    }

    fun sendResponse(id: Long, result: CallbackResult) {
        writeLine(AdapterMessage.CallbackResponse(id, result).encodeNoNewline())
    }

    fun dispose() {
        disposed = true
        disconnect()
    }

    // ── private ───────────────────────────────────────────────────────────────

    private fun writeLine(line: String) {
        try { writer?.println(line) } catch (_: Exception) {}
    }

    private fun disconnect() {
        readJob?.cancel(); readJob = null
        try { socket?.close() } catch (_: Exception) {}
        socket = null; writer = null
    }

    private fun scheduleReconnect() {
        if (disposed) return
        scope.launch { delay(RECONNECT_DELAY_MS); if (!disposed) connect() }
    }

    private suspend fun readLoop(sock: Socket) {
        try {
            val reader = BufferedReader(InputStreamReader(sock.getInputStream(), Charsets.UTF_8))
            while (!sock.isClosed) {
                val line = withContext(Dispatchers.IO) { reader.readLine() } ?: break
                if (line.isBlank()) continue
                val msg = try { decodeServerMessage(line) } catch (e: Exception) {
                    log.warn("CADE: malformed frame — ${e.message}"); continue
                }
                handleServerMessage(msg)
            }
        } catch (_: Exception) {}
        if (!disposed) { log.info("CADE: disconnected — reconnecting…"); scheduleReconnect() }
    }

    private suspend fun handleServerMessage(msg: ServerMessage) {
        when (msg) {
            is ServerMessage.HelloAck ->
                log.info("CADE: HelloAck received (protocol v${msg.protocol_version}). Adapter ready.")
            is ServerMessage.CallbackRequest ->
                handler?.invoke(msg.id, msg.op)
        }
    }

    private object NoOpLogger : CadeLogger {
        override fun info(msg: String) {}
        override fun warn(msg: String) {}
    }
}

private fun AdapterMessage.encodeNoNewline(): String = encode().trimEnd('\n')

// ── Discovery file ────────────────────────────────────────────────────────────

@kotlinx.serialization.Serializable
internal data class DiscoveryInfo(val pid: Int, val addr: String)

internal fun readDiscoveryFile(): DiscoveryInfo? {
    val dir: Path = Paths.get(System.getProperty("user.home"), ".cade", "ide")
    if (!dir.exists()) return null
    return dir.listDirectoryEntries("*.json")
        .mapNotNull { p -> try { p to p.getLastModifiedTime() } catch (_: Exception) { null } }
        .maxByOrNull { it.second }
        ?.first
        ?.let { p -> try { cadeJson.decodeFromString<DiscoveryInfo>(p.readText()) } catch (_: Exception) { null } }
}
