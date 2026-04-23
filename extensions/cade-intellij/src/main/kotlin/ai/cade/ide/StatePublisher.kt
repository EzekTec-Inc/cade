@file:OptIn(ExperimentalSerializationApi::class)

package ai.cade.ide

import com.intellij.codeInsight.daemon.DaemonCodeAnalyzer
import com.intellij.lang.annotation.HighlightSeverity
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.editor.EditorFactory
import com.intellij.openapi.editor.event.DocumentEvent
import com.intellij.openapi.editor.event.DocumentListener
import com.intellij.openapi.editor.event.SelectionEvent
import com.intellij.openapi.editor.event.SelectionListener
import com.intellij.openapi.fileEditor.FileDocumentManager
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerEvent
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.roots.ProjectRootManager
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.psi.PsiDocumentManager
import kotlinx.coroutines.*
import kotlinx.serialization.ExperimentalSerializationApi

private const val DEBOUNCE_MS = 50L

/**
 * StatePublisher — subscribes to IntelliJ Platform events and pushes
 * debounced [StateSnapshot] frames to [CadeConnection].
 *
 * Lifecycle: call [start] once per project; the object registers its
 * own listeners and cleans up when the [Project] is disposed.
 */
class StatePublisher(
    private val project: Project,
    private val connection: CadeConnection,
) : Disposable {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var debounceJob: Job? = null

    fun start() {
        val bus = project.messageBus.connect(this)

        bus.subscribe(FileEditorManagerListener.FILE_EDITOR_MANAGER, object : FileEditorManagerListener {
            override fun fileOpened(source: FileEditorManager, file: VirtualFile) = schedulePublish()
            override fun fileClosed(source: FileEditorManager, file: VirtualFile) = schedulePublish()
            override fun selectionChanged(event: FileEditorManagerEvent) = schedulePublish()
        })

        EditorFactory.getInstance().eventMulticaster.addDocumentListener(object : DocumentListener {
            override fun documentChanged(event: DocumentEvent) = schedulePublish()
        }, this)

        EditorFactory.getInstance().eventMulticaster.addSelectionListener(object : SelectionListener {
            override fun selectionChanged(e: SelectionEvent) = schedulePublish()
        }, this)

        // Initial publish.
        schedulePublish()
    }

    override fun dispose() {
        debounceJob?.cancel()
        scope.cancel()
    }

    private fun schedulePublish() {
        debounceJob?.cancel()
        debounceJob = scope.launch {
            delay(DEBOUNCE_MS)
            withContext(Dispatchers.IO) { publish() }
        }
    }

    private fun publish() {
        val snap = buildSnapshot()
        connection.sendStateUpdate(snap)
    }

    private fun buildSnapshot(): StateSnapshot {
        val fem = FileEditorManager.getInstance(project)
        val fdm = FileDocumentManager.getInstance()
        val pdm = PsiDocumentManager.getInstance(project)

        // Open files.
        val openFiles = fem.openFiles.mapNotNull { vf ->
            val doc = fdm.getDocument(vf) ?: return@mapNotNull null
            val lang = try {
                pdm.getPsiFile(doc)?.language?.id ?: vf.extension ?: "plain"
            } catch (_: Exception) { vf.extension ?: "plain" }
            CadeOpenFile(
                path = vf.path,
                text = doc.text,
                language_id = lang,
                version = doc.modificationStamp.toInt(),
                is_dirty = fdm.isDocumentUnsaved(doc),
            )
        }

        // Active file.
        val activeFile = fem.selectedFiles.firstOrNull()?.path

        // Selection.
        val selection = fem.selectedTextEditor?.let { ed ->
            val sel = ed.selectionModel
            val vf = fdm.getFile(ed.document)
            if (sel.hasSelection() && vf != null) {
                val startOff = sel.selectionStart
                val endOff = sel.selectionEnd
                val doc = ed.document
                CadeSelection(
                    path = vf.path,
                    range = CadeRange(
                        offsetToPosition(doc.text, startOff),
                        offsetToPosition(doc.text, endOff),
                    ),
                    text = sel.selectedText ?: "",
                )
            } else null
        }

        // Diagnostics — read from IntelliJ's highlight info.
        // We use DaemonCodeAnalyzer's file-level problem count as a lightweight proxy.
        // Full diagnostic mapping would require DaemonCodeAnalyzerImpl internals;
        // leave as empty for now (platform-test would need a full IDE instance).
        val diagnostics = emptyList<CadeDiagnostic>()

        // Workspace folders.
        val roots = ProjectRootManager.getInstance(project).contentRoots
        val workspaceFolders = roots.map { CadeWorkspaceFolder(it.path, it.name) }

        // Visible range.
        val visibleRange = fem.selectedTextEditor?.let { ed ->
            val area = ed.scrollingModel.visibleArea
            val startLine = ed.document.getLineNumber(
                ed.logicalPositionToOffset(ed.xyToLogicalPosition(area.location))
            )
            val endLine = ed.document.getLineNumber(
                ed.logicalPositionToOffset(ed.xyToLogicalPosition(
                    java.awt.Point(area.x, area.y + area.height - 1)
                ))
            ).coerceAtMost(ed.document.lineCount - 1)
            listOf(startLine, endLine)
        }

        return StateSnapshot(
            open_files = openFiles,
            active_file = activeFile,
            selection = selection,
            diagnostics = diagnostics,
            workspace_folders = workspaceFolders,
            visible_range = visibleRange,
        )
    }

    private fun offsetToPosition(text: String, offset: Int): CadePosition {
        var line = 0; var col = 0
        for (i in 0 until offset.coerceAtMost(text.length)) {
            if (text[i] == '\n') { line++; col = 0 } else col++
        }
        return CadePosition(line, col)
    }
}
