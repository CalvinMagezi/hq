package example.hq.spike.ui.shell

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.data.HqUnavailable
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class UndoTest {
    private class Fake(val status: Int? = null) : UndoRestore {
        val calls = mutableListOf<String>()
        private fun hit(s: String) { calls += s; status?.let { throw HqUnavailable("no", status = it) } }
        override suspend fun unarchiveThread(id: String) = hit("thread:$id")
        override suspend fun restoreNotification(id: String) = hit("note:$id")
        override suspend fun restoreTask(id: String) = hit("task:$id")
        override suspend fun restoreBudgets(limits: JSONObject) = hit("budgets")
    }

    @Test fun tenSecondsBeforeAndroid10() {
        assertEquals(10_000L, Undo.millis(28) { 99_000 })
    }

    @Test fun thePhonesOwnTimeoutFromAndroid10ButNeverUnderFive() {
        assertEquals(12_000L, Undo.millis(29) { 12_000 })
        assertEquals(5_000L, Undo.millis(29) { 1_000 })
    }

    @Test fun eachKindUsesItsOwnRoute() = runBlocking {
        val f = Fake()
        assertEquals(UndoResult.Done, Undo.restore(f, UndoKind.Thread, "t1"))
        Undo.restore(f, UndoKind.Notification, "n1")
        Undo.restore(f, UndoKind.Task, "k1")
        Undo.restore(f, UndoKind.Budgets, "", JSONObject())
        assertEquals(listOf("thread:t1", "note:n1", "task:k1", "budgets"), f.calls)
    }

    @Test fun a409IsTooLateAndShowsTheLine() = runBlocking {
        val context: Context = ApplicationProvider.getApplicationContext()
        val result = Undo.restore(Fake(409), UndoKind.Thread, "t1")
        assertEquals(UndoResult.TooLate, result)
        assertEquals(context.getString(example.hq.spike.R.string.fail_undo_too_late), Undo.failLine(context, result))
    }

    @Test fun anOutageIsNotTooLate() = runBlocking {
        val context: Context = ApplicationProvider.getApplicationContext()
        val result = Undo.restore(Fake(503), UndoKind.Task, "k1")
        assertEquals(UndoResult.Unreachable, result)
        assertNull(Undo.failLine(context, result))
    }
}
