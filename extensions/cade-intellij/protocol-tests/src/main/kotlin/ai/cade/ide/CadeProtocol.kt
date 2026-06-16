@file:OptIn(ExperimentalSerializationApi::class)

package ai.cade.ide

import kotlinx.serialization.*
import kotlinx.serialization.json.*

// ── Shared sub-types ──────────────────────────────────────────────────────────

@Serializable
data class CadePosition(val line: Int, val character: Int)

@Serializable
data class CadeRange(val start: CadePosition, val end: CadePosition)

@Serializable
data class CadeOpenFile(
    val path: String?,
    val text: String,
    val language_id: String,
    val version: Int,
    val is_dirty: Boolean,
)

@Serializable
data class CadeSelection(val path: String, val range: CadeRange, val text: String)

@Serializable
data class CadeDiagnostic(
    val path: String,
    val range: CadeRange,
    val severity: String,   // "error" | "warning" | "info" | "hint"
    val message: String,
    val source: String?,
    val code: String?,
)

@Serializable
data class CadeWorkspaceFolder(val path: String, val name: String)

@Serializable
data class CadeTextEdit(val range: CadeRange, val new_text: String)

@Serializable
data class CadeApplyEditRequest(val path: String, val text_edits: List<CadeTextEdit>)

/** Full editor-state snapshot. */
@Serializable
data class StateSnapshot(
    val open_files: List<CadeOpenFile>,
    val active_file: String?,
    val selection: CadeSelection?,
    val diagnostics: List<CadeDiagnostic>,
    val workspace_folders: List<CadeWorkspaceFolder>,
    val visible_range: List<Int>?,   // [start, end] or null
)

// ── Adapter → server messages ─────────────────────────────────────────────────

@Serializable
@JsonClassDiscriminator("type")
sealed class AdapterMessage {

    @Serializable
    @SerialName("hello")
    data class Hello(val label: String, val protocol_version: Int) : AdapterMessage()

    @Serializable
    @SerialName("state_update")
    data class StateUpdate(
        val open_files: List<CadeOpenFile>,
        val active_file: String?,
        val selection: CadeSelection?,
        val diagnostics: List<CadeDiagnostic>,
        val workspace_folders: List<CadeWorkspaceFolder>,
        val visible_range: List<Int>?,
    ) : AdapterMessage()

    @Serializable
    @SerialName("callback_response")
    data class CallbackResponse(
        val id: Long,
        val result: CallbackResult,
    ) : AdapterMessage()
}

@Serializable
@JsonClassDiscriminator("status")
sealed class CallbackResult {
    @Serializable @SerialName("ok") data object Ok : CallbackResult()
    @Serializable @SerialName("err") data class Err(val message: String) : CallbackResult()
}

// ── Server → adapter messages ─────────────────────────────────────────────────

@Serializable
@JsonClassDiscriminator("type")
sealed class ServerMessage {

    @Serializable
    @SerialName("hello_ack")
    data class HelloAck(val protocol_version: Int) : ServerMessage()

    @Serializable
    @SerialName("callback_request")
    data class CallbackRequest(val id: Long, val op: CallbackOp) : ServerMessage()
}

@Serializable
@JsonClassDiscriminator("op")
sealed class CallbackOp {
    @Serializable @SerialName("apply_edit")
    data class ApplyEdit(val path: String, val text_edits: List<CadeTextEdit>) : CallbackOp()

    @Serializable @SerialName("reveal_file")
    data class RevealFile(val path: String) : CallbackOp()

    @Serializable @SerialName("set_selection")
    data class SetSelection(val path: String, val range: CadeRange) : CallbackOp()

    @Serializable @SerialName("save")
    data class Save(val path: String?) : CallbackOp()

    @Serializable @SerialName("run_task")
    data class RunTask(val name: String) : CallbackOp()

    @Serializable @SerialName("run_terminal")
    data class RunTerminal(val command: String) : CallbackOp()

    @Serializable @SerialName("debug_control")
    data class DebugControl(val action: String, val config: String? = null) : CallbackOp()
}

// ── JSON codec ────────────────────────────────────────────────────────────────

val cadeJson = Json {
    ignoreUnknownKeys = true
    encodeDefaults = true
    classDiscriminator = "type"   // default; overridden per hierarchy above
}

/** Encode an [AdapterMessage] as a single JSON line (no embedded newlines) + "\n". */
fun AdapterMessage.encode(): String = cadeJson.encodeToString(AdapterMessage.serializer(), this) + "\n"

/** Decode a newline-delimited JSON line into a [ServerMessage]. */
fun decodeServerMessage(line: String): ServerMessage = cadeJson.decodeFromString(line.trim())
