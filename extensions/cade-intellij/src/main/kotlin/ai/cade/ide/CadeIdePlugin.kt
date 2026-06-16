package ai.cade.ide

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.Service
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

private val LOG = logger<CadeConnectionService>()

// ── Application-level service that owns the connection lifecycle ──────────────

@Service(Service.Level.APP)
class CadeConnectionService : AutoCloseable {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    val connection = CadeConnection(scope)

    fun connect() {
        scope.launch { connection.connect() }
    }

    override fun close() {
        connection.dispose()
    }

    companion object {
        fun getInstance(): CadeConnectionService =
            ApplicationManager.getApplication()
                .getService(CadeConnectionService::class.java)
    }
}

// ── Project startup activity ──────────────────────────────────────────────────

class CadePostStartupActivity : ProjectActivity {
    override suspend fun execute(project: Project) {
        LOG.info("CADE IDE Bridge: starting up for project ${project.name}")
        val service = CadeConnectionService.getInstance()
        val publisher = StatePublisher(project, service.connection)
        service.connection.onCallbackRequest { id, op ->
            val handler = CallbackHandler(project)
            val result = handler.handle(op)
            service.connection.sendResponse(id, result)
        }
        publisher.start()
        service.connect()
    }
}

// ── Reconnect action ──────────────────────────────────────────────────────────

class ReconnectAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        LOG.info("CADE IDE Bridge: manual reconnect requested")
        CadeConnectionService.getInstance().connect()
    }
}
