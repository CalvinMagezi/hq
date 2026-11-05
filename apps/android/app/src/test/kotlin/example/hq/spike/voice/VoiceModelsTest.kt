package example.hq.spike.voice

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.R
import example.hq.spike.download.DlState
import example.hq.spike.download.Downloads
import example.hq.spike.measure.DataPurpose
import example.hq.spike.testing.testWords
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class VoiceModelsTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()

    @Test fun theModelIsOneJobOnTheSharedEngineForTheSpeechPurpose() {
        val item = VoiceModels(context).item()
        assertEquals(VoiceModels.ITEM_ID, item.id)
        assertEquals(DataPurpose.SpeechModel, item.purpose)
        assertTrue(item.speech)
        assertEquals(VoiceModels.SIZE_BYTES, item.size)
        assertTrue(item.url.startsWith("https://"))
    }

    @Test fun withoutAnArchiveNothingIsFinishedAndNothingIsReady() {
        val models = VoiceModels(context)
        assertFalse(models.ready())
        assertFalse(models.finish())
        assertEquals(0L, models.onPhoneBytes())
    }

    @Test fun anArchiveThatCannotBeOpenedLeavesNoModelBehind() {
        val models = VoiceModels(context)
        val archive = File(Downloads.dirOf(context), "speech-model.tar.bz2").apply { parentFile?.mkdirs(); writeText("not an archive") }
        assertFalse(models.finish())
        assertFalse(models.ready())
        assertFalse(File(models.stt, "encoder.onnx").exists())
        archive.delete()
    }

    @Test fun deletingIsSafeWhenThereIsNothingToDelete() {
        val models = VoiceModels(context)
        models.delete()
        assertFalse(models.ready())
    }

    @Test fun everyPhaseHasItsOwnWordsAndAskIsOnlyOfferedWhenThereIsAModel() {
        val w = testWords()
        for (phase in VoicePhase.entries) {
            assertTrue(VoiceWords.label(w, phase).isNotBlank())
            assertTrue(VoiceWords.action(w, phase).isNotBlank())
            VoiceWords.body(w, phase, DlState.Ready)
        }
        assertEquals(w.s(R.string.voice_act_wait), VoiceWords.action(w, VoicePhase.NeedsModel))
        assertEquals(w.s(R.string.voice_act_ask), VoiceWords.action(w, VoicePhase.ModelReady))
        assertEquals(w.s(R.string.speech_waiting_body), VoiceWords.body(w, VoicePhase.Downloading, DlState.WaitingWifi))
        assertEquals(null, VoiceWords.body(w, VoicePhase.Listening, DlState.Ready))
        assertNotNull(VoiceWords.body(w, VoicePhase.ModelReady, DlState.Ready))
    }
}
