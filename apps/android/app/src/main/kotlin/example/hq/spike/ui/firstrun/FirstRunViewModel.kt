package example.hq.spike.ui.firstrun

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.HubService
import example.hq.spike.KeyVault
import example.hq.spike.R
import java.io.File
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** What the first-run screens show. Credentials typed here live in memory until Next seals them, and are never saved to a file. */
data class FirstRunUi(
    val step: Step? = null,
    val provider: Provider = Provider.OpenRouter,
    val key: String = "",
    val showKey: Boolean = false,
    val baseUrl: String = "",
    val modelName: String = "",
    val telegram: String = "",
    val discord: String = "",
    val error: String? = null,
    val checking: Boolean = false,
    val checkResult: String? = null,
    val busy: Boolean = false,
    val backup: Uri? = null,
    val backupName: String? = null,
    val useKeyFile: Boolean = false,
    val passphrase: String = "",
    val keyFile: Uri? = null,
    val keyFileName: String? = null,
    val restoring: Boolean = false,
    val restored: Boolean = false,
    val moved: Boolean = false,
    val movedFacts: MovedFacts? = null,
    val movedChecking: Boolean = false,
    val movedForgot: Boolean = false,
    val confirmFresh: Boolean = false,
)

class FirstRunViewModel(app: Application, initial: Step?) : AndroidViewModel(app) {
    private val store = FirstRunStore(app)
    private val mutableUi = MutableStateFlow(FirstRunUi(step = initial))
    val ui: StateFlow<FirstRunUi> = mutableUi.asStateFlow()

    init {
        if (store.moved()) {
            mutableUi.update { it.copy(moved = true) }
            if (initial != null && initial != Step.Welcome && initial != Step.Restore) prepareMoved(notesOnly = false)
        }
    }

    val actions = FirstRunActions(
        onNewHq = { go(Step.Model) },
        onRestore = { go(Step.Restore) },
        onBack = { go(FirstRunLogic.back(mutableUi.value.step ?: Step.Welcome, mutableUi.value.moved)) },
        onProvider = { p -> mutableUi.update { it.copy(provider = p, error = null, checkResult = null) } },
        onKey = { v -> mutableUi.update { it.copy(key = v, error = null, checkResult = null) } },
        onShowKey = { mutableUi.update { it.copy(showKey = !it.showKey) } },
        onBaseUrl = { v -> mutableUi.update { it.copy(baseUrl = v, error = null, checkResult = null) } },
        onModelName = { v -> mutableUi.update { it.copy(modelName = v, error = null) } },
        onCheck = ::check,
        onModelNext = ::modelNext,
        onTelegram = { v -> mutableUi.update { it.copy(telegram = v, error = null) } },
        onDiscord = { v -> mutableUi.update { it.copy(discord = v, error = null) } },
        onChatNext = { chatNext(skip = false) },
        onChatSkip = { chatNext(skip = true) },
        onStart = ::start,
        onBackup = { uri, name -> mutableUi.update { it.copy(backup = uri, backupName = name, error = null) } },
        onUseKeyFile = { on -> mutableUi.update { it.copy(useKeyFile = on, error = null) } },
        onPassphrase = { v -> mutableUi.update { it.copy(passphrase = v, error = null) } },
        onKeyFile = { uri, name -> mutableUi.update { it.copy(keyFile = uri, keyFileName = name, error = null) } },
        onRunRestore = ::restore,
        onContinue = { go(FirstRunLogic.next(Step.Restore)) },
        onAddKeys = { go(Step.Model) },
        onStartFresh = { mutableUi.update { it.copy(confirmFresh = true) } },
        onKeepNotes = { mutableUi.update { it.copy(confirmFresh = false) } },
        onDeleteMoved = ::deleteMoved,
        onMovedRestore = { go(Step.Restore) },
        onNotesOnly = { prepareMoved(notesOnly = true) },
    )

    /**
     * Before HQ runs on a copied vault: checks the database, then makes HQ forget what belonged to the old phone.
     * hq is held stopped meanwhile, so nothing has the database open. A broken database stops here and shows frame 7.
     */
    private fun prepareMoved(notesOnly: Boolean) {
        mutableUi.update { it.copy(movedChecking = true, error = null) }
        viewModelScope.launch {
            val app = getApplication<Application>()
            val result = withContext(Dispatchers.IO) {
                val held = HubService.holdHq()
                try {
                    if (!held) return@withContext null to false
                    val vault = Moved.vault(app)
                    if (notesOnly) MovedPhone.dropDatabase(vault)
                    val facts = MovedPhone.facts(vault)
                    facts to (facts.databaseOk && MovedPhone.forget(app))
                } finally {
                    if (held) HubService.releaseHq()
                }
            }
            val (facts, forgot) = result
            mutableUi.update {
                it.copy(
                    movedChecking = false, movedFacts = facts ?: it.movedFacts, movedForgot = forgot,
                    error = if (facts != null && facts.databaseOk && !forgot) app.getString(R.string.moved_forget_failed) else null,
                )
            }
            if (notesOnly && forgot) go(Step.Model)
            else if (facts != null && !facts.databaseOk && mutableUi.value.step != Step.Moved) go(Step.Moved)
        }
    }

    private fun deleteMoved() {
        mutableUi.update { it.copy(busy = true, confirmFresh = false) }
        viewModelScope.launch {
            val app = getApplication<Application>()
            withContext(Dispatchers.IO) {
                val held = HubService.holdHq()
                try {
                    Moved.vault(app).deleteRecursively()
                } finally {
                    if (held) HubService.releaseHq()
                }
            }
            store.forgetMoved()
            mutableUi.update { it.copy(busy = false, moved = false, movedFacts = null, movedForgot = false) }
            go(Step.Welcome)
        }
    }

    private fun go(step: Step) {
        store.save(step)
        mutableUi.update { it.copy(step = step, error = null, checkResult = null, restored = false) }
    }

    private fun check() {
        val s = mutableUi.value
        val problem = KeyShape.keyProblem(s.key) ?: if (s.provider.needsBaseUrl) KeyShape.baseUrlProblem(s.baseUrl) else null
        if (problem != null) {
            mutableUi.update { it.copy(error = problem) }
            return
        }
        mutableUi.update { it.copy(checking = true, error = null, checkResult = null) }
        viewModelScope.launch {
            val status = KeyCheck.status(KeyCheck.request(s.provider, s.key, s.baseUrl))
            mutableUi.update { it.copy(checking = false, checkResult = KeyCheck.message(status)) }
        }
    }

    private fun modelNext() {
        val s = mutableUi.value
        if (s.key.isEmpty()) {
            // Coming back to this step with a key already sealed: nothing to paste again.
            viewModelScope.launch {
                val stored = withContext(Dispatchers.IO) { KeyVault.hasModelKey(getApplication<Application>()) }
                if (stored) go(Step.Chat) else mutableUi.update { it.copy(error = FirstRunWords.ERR_KEY_EMPTY) }
            }
            return
        }
        val problem = KeyShape.keyProblem(s.key)
            ?: if (s.provider.needsBaseUrl) KeyShape.baseUrlProblem(s.baseUrl) ?: KeyShape.modelProblem(s.modelName) else null
        if (problem != null) {
            mutableUi.update { it.copy(error = problem) }
            return
        }
        mutableUi.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            val failure: String? = withContext(Dispatchers.IO) {
                val config: String? = if (s.provider.needsBaseUrl) ConfigYaml.apply(File(getApplication<Application>().filesDir, "hqhome/config.yaml"), s.baseUrl, s.modelName, s.provider.credential) else null
                if (config != null) config
                else if (KeyVault.put(getApplication(), s.provider.credential, s.key) != null) FirstRunWords.ERR_NOT_SAVED
                else null
            }
            if (failure == null) {
                mutableUi.update { it.copy(busy = false, key = "", checkResult = null) }
                go(Step.Chat)
            } else {
                mutableUi.update { it.copy(busy = false, error = failure) }
            }
        }
    }

    private fun chatNext(skip: Boolean) {
        val s = mutableUi.value
        val tokens = if (skip) emptyList() else listOf(TELEGRAM to s.telegram, DISCORD to s.discord).filter { it.second.isNotEmpty() }
        val problem = tokens.firstNotNullOfOrNull { KeyShape.keyProblem(it.second) }
        if (problem != null) {
            mutableUi.update { it.copy(error = problem) }
            return
        }
        if (tokens.isEmpty()) {
            go(Step.Start)
            return
        }
        mutableUi.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            val failed = withContext(Dispatchers.IO) { tokens.any { (name, value) -> KeyVault.put(getApplication(), name, value) != null } }
            if (failed) {
                mutableUi.update { it.copy(busy = false, error = FirstRunWords.ERR_NOT_SAVED) }
            } else {
                mutableUi.update { it.copy(busy = false, telegram = "", discord = "") }
                go(Step.Start)
            }
        }
    }

    private fun start() {
        mutableUi.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            val app = getApplication<Application>()
            val s = mutableUi.value
            // HQ must not run on a copied vault until the old phone's sign-ins are cleared.
            if (s.moved && !s.movedForgot && !withContext(Dispatchers.IO) { MovedPhone.forget(app) }) {
                mutableUi.update { it.copy(busy = false, error = app.getString(R.string.moved_forget_failed)) }
                return@launch
            }
            val failed = withContext(Dispatchers.IO) {
                try {
                    HubService.switchMode(getApplication(), true)
                    false
                } catch (_: IOException) {
                    true
                }
            }
            if (failed) {
                mutableUi.update { it.copy(busy = false, error = "The run mode could not be saved. Try again.") }
            } else {
                store.finish()
                mutableUi.update { it.copy(busy = false, step = null, moved = false) }
            }
        }
    }

    private fun restore() {
        val s = mutableUi.value
        val backup = s.backup
        val secret: BackupSecret? = when {
            backup == null -> null
            s.useKeyFile -> s.keyFile?.let { BackupSecret.KeyFile(it) }
            else -> BackupSecret.Passphrase(s.passphrase)
        }
        val problem = when {
            backup == null -> FirstRunWords.RESTORE_NEEDS_FILE
            s.useKeyFile && s.keyFile == null -> FirstRunWords.RESTORE_NEEDS_KEY
            !s.useKeyFile -> RestoreLogic.passphraseProblem(s.passphrase)
            else -> null
        }
        if (problem != null || backup == null || secret == null) {
            mutableUi.update { it.copy(error = problem ?: FirstRunWords.RESTORE_NEEDS_FILE) }
            return
        }
        mutableUi.update { it.copy(restoring = true, error = null) }
        viewModelScope.launch {
            val outcome = Restore.run(getApplication(), backup, secret)
            mutableUi.update {
                when (outcome) {
                    RestoreOutcome.Done -> it.copy(restoring = false, restored = true, passphrase = "", keyFile = null, keyFileName = null)
                    is RestoreOutcome.Failed -> it.copy(restoring = false, error = FirstRunWords.restoreFailedWith(outcome.reason))
                }
            }
            if (outcome == RestoreOutcome.Done) store.save(Step.Start)
        }
    }

    private companion object {
        const val TELEGRAM = "HQ_RELAY__TELEGRAM_TOKEN"
        const val DISCORD = "HQ_RELAY__DISCORD_TOKEN"
    }
}

class FirstRunActions(
    val onNewHq: () -> Unit,
    val onRestore: () -> Unit,
    val onBack: () -> Unit,
    val onProvider: (Provider) -> Unit,
    val onKey: (String) -> Unit,
    val onShowKey: () -> Unit,
    val onBaseUrl: (String) -> Unit,
    val onModelName: (String) -> Unit,
    val onCheck: () -> Unit,
    val onModelNext: () -> Unit,
    val onTelegram: (String) -> Unit,
    val onDiscord: (String) -> Unit,
    val onChatNext: () -> Unit,
    val onChatSkip: () -> Unit,
    val onStart: () -> Unit,
    val onBackup: (Uri, String?) -> Unit,
    val onUseKeyFile: (Boolean) -> Unit,
    val onPassphrase: (String) -> Unit,
    val onKeyFile: (Uri, String?) -> Unit,
    val onRunRestore: () -> Unit,
    val onContinue: () -> Unit,
    val onAddKeys: () -> Unit = {},
    val onStartFresh: () -> Unit = {},
    val onKeepNotes: () -> Unit = {},
    val onDeleteMoved: () -> Unit = {},
    val onMovedRestore: () -> Unit = {},
    val onNotesOnly: () -> Unit = {},
)
