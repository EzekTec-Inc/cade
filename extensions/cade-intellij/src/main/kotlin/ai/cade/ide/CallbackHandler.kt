@file:OptIn(ExperimentalSerializationApi::class)

package ai.cade.ide

import com.intellij.execution.executors.DefaultRunExecutor
import com.intellij.execution.runners.ExecutionEnvironmentBuilder
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.OpenFileDescriptor
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.vfs.VirtualFileManager
import com.intellij.openapi.wm.ToolWindowManager
import com.intellij.terminal.JBTerminalWidget
import com.intellij.xdebugger.XDebuggerManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.ExperimentalSerializationApi

/**
 * CallbackHandler — receives a [CallbackOp] and executes the
 * corresponding IntelliJ Platform operation.
 *
 * Returns a [CallbackResult] indicating success or the error message.
 */
class CallbackHandler(private val project: Project) {

    suspend fun handle(op: CallbackOp): CallbackResult = try {
        dispatch(op)
        CallbackResult.Ok
    } catch (e: Exception) {
        CallbackResult.Err(e.message ?: e.toString())
    }

    private suspend fun dispatch(op: CallbackOp): Unit = when (op) {
        is CallbackOp.ApplyEdit    -> applyEdit(op)
        is CallbackOp.RevealFile   -> revealFile(op)
        is CallbackOp.SetSelection -> setSelection(op)
        is CallbackOp.Save         -> save(op)
        is CallbackOp.RunTask      -> runTask(op)
        is CallbackOp.RunTerminal  -> runTerminal(op)
        is CallbackOp.DebugControl -> debugControl(op)
    }

    // ── apply_edit ────────────────────────────────────────────────────────────

    private suspend fun applyEdit(op: CallbackOp.ApplyEdit): Unit = withContext(Dispatchers.Main) {
        val vf = LocalFileSystem.getInstance().findFileByPath(op.path)
            ?: error("file not found: ${op.path}")
        val doc = com.intellij.openapi.fileEditor.FileDocumentManager.getInstance()
            .getDocument(vf)
            ?: error("could not get document for ${op.path}")

        WriteCommandAction.runWriteCommandAction(project) {
            for (edit in op.text_edits.sortedByDescending { it.range.start.line * 100_000 + it.range.start.character }) {
                val startOff = positionToOffset(doc.text, edit.range.start)
                val endOff   = positionToOffset(doc.text, edit.range.end)
                doc.replaceString(startOff, endOff, edit.new_text)
            }
        }
    }

    // ── reveal_file ───────────────────────────────────────────────────────────

    private suspend fun revealFile(op: CallbackOp.RevealFile): Unit = withContext(Dispatchers.Main) {
        val vf = LocalFileSystem.getInstance().refreshAndFindFileByPath(op.path)
            ?: error("file not found: ${op.path}")
        FileEditorManager.getInstance(project).openFile(vf, true)
    }

    // ── set_selection ─────────────────────────────────────────────────────────

    private suspend fun setSelection(op: CallbackOp.SetSelection): Unit = withContext(Dispatchers.Main) {
        val vf = LocalFileSystem.getInstance().findFileByPath(op.path)
            ?: error("file not found: ${op.path}")
        val editor = FileEditorManager.getInstance(project)
            .openTextEditor(OpenFileDescriptor(project, vf), true)
            ?: error("could not open editor for ${op.path}")

        val doc = editor.document
        val startOff = positionToOffset(doc.text, op.range.start)
        val endOff   = positionToOffset(doc.text, op.range.end)
        editor.selectionModel.setSelection(startOff, endOff)
        editor.caretModel.moveToOffset(endOff)
    }

    // ── save ──────────────────────────────────────────────────────────────────

    private suspend fun save(op: CallbackOp.Save): Unit = withContext(Dispatchers.Main) {
        val fdm = com.intellij.openapi.fileEditor.FileDocumentManager.getInstance()
        if (op.path == null) {
            ApplicationManager.getApplication().invokeAndWait { fdm.saveAllDocuments() }
        } else {
            val vf = LocalFileSystem.getInstance().findFileByPath(op.path)
                ?: error("file not found: ${op.path}")
            val doc = fdm.getDocument(vf) ?: error("could not get document for ${op.path}")
            ApplicationManager.getApplication().invokeAndWait { fdm.saveDocument(doc) }
        }
    }

    // ── run_task ──────────────────────────────────────────────────────────────

    private suspend fun runTask(op: CallbackOp.RunTask): Unit = withContext(Dispatchers.Main) {
        // Delegate to "Run Anything" by name via the run configuration manager.
        // This is a best-effort implementation; full integration would require
        // parsing the run configuration list, which varies by platform version.
        val mgr = com.intellij.execution.RunManager.getInstance(project)
        val settings = mgr.allSettings.find { it.name == op.name }
            ?: error("run configuration '${op.name}' not found")
        val env = ExecutionEnvironmentBuilder.create(
            DefaultRunExecutor.getRunExecutorInstance(), settings
        ).build()
        com.intellij.execution.ProgramRunnerUtil.executeConfiguration(env, false, true)
    }

    // ── run_terminal ──────────────────────────────────────────────────────────

    private suspend fun runTerminal(op: CallbackOp.RunTerminal): Unit = withContext(Dispatchers.Main) {
        val twm = ToolWindowManager.getInstance(project)
        val tw = twm.getToolWindow("Terminal")
            ?: error("Terminal tool window not found — is the Terminal plugin enabled?")
        tw.show()
        // Send the command to the active terminal widget if available.
        val widget = tw.contentManager.selectedContent
            ?.component
            ?.let { com.intellij.ui.content.ContentManager::class.java.cast(it) } as? JBTerminalWidget
        if (widget != null) {
            widget.terminalStarter?.sendString(op.command + "\n", true)
        }
        // Fallback: open a new session via the TerminalView (plugin API).
    }

    // ── debug_control ─────────────────────────────────────────────────────────

    private suspend fun debugControl(op: CallbackOp.DebugControl): Unit = withContext(Dispatchers.Main) {
        val xdm = XDebuggerManager.getInstance(project)
        when (op.action) {
            "start" -> {
                // Trigger debug via the run configuration of the given name.
                val mgr = com.intellij.execution.RunManager.getInstance(project)
                val settings = mgr.allSettings.find { it.name == op.config }
                    ?: error("run configuration '${op.config}' not found")
                val env = ExecutionEnvironmentBuilder.create(
                    com.intellij.execution.executors.DefaultDebugExecutor.getDebugExecutorInstance(),
                    settings,
                ).build()
                com.intellij.execution.ProgramRunnerUtil.executeConfiguration(env, false, true)
            }
            "stop" -> xdm.debugSessions.forEach { it.stop() }
            else   -> error("unknown debug action '${op.action}'")
        }
    }

    // ── utilities ─────────────────────────────────────────────────────────────

    private fun positionToOffset(text: String, pos: CadePosition): Int {
        var line = 0; var idx = 0
        while (idx < text.length && line < pos.line) {
            if (text[idx++] == '\n') line++
        }
        return (idx + pos.character).coerceAtMost(text.length)
    }
}
