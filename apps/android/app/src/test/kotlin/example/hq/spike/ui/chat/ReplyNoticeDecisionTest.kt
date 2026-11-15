package example.hq.spike.ui.chat

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ReplyNoticeDecisionTest {
    @Test
    fun notifiesWhenAppIsInBackground() {
        assertTrue(shouldNotifyReply(appVisible = false, inChatPlace = true, openThreadId = "t1", replyThreadId = "t1", stopped = false))
    }

    @Test
    fun notifiesWhenAnotherTabIsOpen() {
        assertTrue(shouldNotifyReply(appVisible = true, inChatPlace = false, openThreadId = "t1", replyThreadId = "t1", stopped = false))
    }

    @Test
    fun notifiesWhenAnotherThreadOrNewChatIsOpen() {
        assertTrue(shouldNotifyReply(appVisible = true, inChatPlace = true, openThreadId = "t2", replyThreadId = "t1", stopped = false))
        assertTrue(shouldNotifyReply(appVisible = true, inChatPlace = true, openThreadId = null, replyThreadId = "t1", stopped = false))
    }

    @Test
    fun silentWhenViewingThatThreadInForeground() {
        assertFalse(shouldNotifyReply(appVisible = true, inChatPlace = true, openThreadId = "t1", replyThreadId = "t1", stopped = false))
    }

    @Test
    fun silentWhenStoppedWaitingOrMissingThread() {
        assertFalse(shouldNotifyReply(appVisible = false, inChatPlace = false, openThreadId = "t2", replyThreadId = "t1", stopped = true))
        assertFalse(shouldNotifyReply(appVisible = false, inChatPlace = false, openThreadId = "t2", replyThreadId = "t1", stopped = false, waitingForOwner = true))
        assertFalse(shouldNotifyReply(appVisible = false, inChatPlace = false, openThreadId = "t2", replyThreadId = null, stopped = false))
    }
}
