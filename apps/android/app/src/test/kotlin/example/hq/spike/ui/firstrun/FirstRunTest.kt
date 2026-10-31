package example.hq.spike.ui.firstrun

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FirstRunLogicTest {
    @Test fun aCleanInstallStartsAtWelcome() {
        assertEquals(Step.Welcome, FirstRunLogic.decide(done = false, saved = null, hasModelKey = false, heardHqBefore = false))
    }

    @Test fun aFinishedFlowStaysFinishedWhateverElseIsStored() {
        assertNull(FirstRunLogic.decide(done = true, saved = Step.Model, hasModelKey = false, heardHqBefore = false))
        assertNull(FirstRunLogic.decide(done = true, saved = null, hasModelKey = false, heardHqBefore = false))
    }

    @Test fun anUpgradeOrAnAdbSetupIsNotAskedAgain() {
        assertNull(FirstRunLogic.decide(done = false, saved = null, hasModelKey = true, heardHqBefore = false))
        assertNull(FirstRunLogic.decide(done = false, saved = null, hasModelKey = false, heardHqBefore = true))
    }

    @Test fun aKillResumesTheSavedStepEvenThoughKeyAndFirstAnswerNowExist() {
        // The key is sealed at step 1 and hq answers /health during the flow: neither may end it early.
        for (step in listOf(Step.Welcome, Step.Model, Step.Chat, Step.Start, Step.Restore)) {
            assertEquals(step, FirstRunLogic.decide(done = false, saved = step, hasModelKey = true, heardHqBefore = true))
        }
    }

    @Test fun eachButtonLeadsToTheNextScreen() {
        assertEquals(Step.Model, FirstRunLogic.next(Step.Welcome))
        assertEquals(Step.Chat, FirstRunLogic.next(Step.Model))
        assertEquals(Step.Start, FirstRunLogic.next(Step.Chat))
        assertEquals(Step.Start, FirstRunLogic.next(Step.Restore))
        assertEquals(Step.Welcome, FirstRunLogic.back(Step.Model))
        assertEquals(Step.Model, FirstRunLogic.back(Step.Chat))
        assertEquals(Step.Chat, FirstRunLogic.back(Step.Start))
        assertEquals(Step.Welcome, FirstRunLogic.back(Step.Restore))
    }
}

class KeyShapeTest {
    @Test fun aKeyNeedsTextAndNoWhitespace() {
        assertNull(KeyShape.keyProblem("sk-or-v1-abc123"))
        assertEquals(FirstRunWords.ERR_KEY_EMPTY, KeyShape.keyProblem(""))
        assertEquals(FirstRunWords.ERR_KEY_SPACES, KeyShape.keyProblem("sk or"))
        assertEquals(FirstRunWords.ERR_KEY_SPACES, KeyShape.keyProblem("sk-or\n"))
        assertEquals(FirstRunWords.ERR_KEY_SPACES, KeyShape.keyProblem("sk-or\u0001"))
    }

    @Test fun aBaseUrlIsHttpWithAHostAndNoLogin() {
        assertNull(KeyShape.baseUrlProblem("https://example.com/v1"))
        assertNull(KeyShape.baseUrlProblem("  http://example.com:8080  "))
        assertEquals(FirstRunWords.ERR_URL_EMPTY, KeyShape.baseUrlProblem("  "))
        assertEquals(FirstRunWords.ERR_URL_SHAPE, KeyShape.baseUrlProblem("example.com/v1"))
        assertEquals(FirstRunWords.ERR_URL_SHAPE, KeyShape.baseUrlProblem("ftp://example.com"))
        assertEquals(FirstRunWords.ERR_URL_SHAPE, KeyShape.baseUrlProblem("https://"))
        assertEquals(FirstRunWords.ERR_URL_SHAPE, KeyShape.baseUrlProblem("https://exa mple.com"))
        assertEquals(FirstRunWords.ERR_URL_SHAPE, KeyShape.baseUrlProblem("https://example.com/\"x"))
        assertEquals(FirstRunWords.ERR_URL_LOGIN, KeyShape.baseUrlProblem("https://user:pw@example.com/v1"))
        assertNull(KeyShape.baseUrlProblem("https://example.com/path@x"))
    }

    @Test fun theCheckAsksTheProvidersOwnCheapEndpoint() {
        val or = KeyCheck.request(Provider.OpenRouter, "k")
        assertEquals("https://openrouter.ai/api/v1/auth/key", or.url)
        assertEquals("Bearer k", or.headers["Authorization"])
        val an = KeyCheck.request(Provider.Anthropic, "k")
        assertEquals("https://api.anthropic.com/v1/models", an.url)
        assertEquals("k", an.headers["x-api-key"])
        assertEquals("https://example.com/v1/models", KeyCheck.request(Provider.Other, "k", " https://example.com/v1/ ").url)
        assertEquals(FirstRunWords.KEY_OK, KeyCheck.message(200))
        assertEquals("The service answered 401.", KeyCheck.message(401))
        assertTrue(KeyCheck.message(null).startsWith("Could not reach"))
    }

    @Test fun everyProviderStoresUnderANameHqAcceptsAsAKey() {
        Provider.entries.forEach { assertTrue(it.credential, it.credential.endsWith("_API_KEY")) }
    }
}

class ConfigYamlTest {
    @Test fun anOtherServiceBecomesTheOnlyBackendInAnEmptyFile() {
        val r = ConfigYaml.withBackend("", "https://example.com/v1/", "", "CUSTOM_API_KEY") as ConfigYaml.Result.Written
        assertTrue(r.text.contains("backends:\n  primary: custom\n  backends:\n    - name: custom\n      kind: openai-compatible\n      endpoint: \"https://example.com/v1\"\n      credential_env: CUSTOM_API_KEY\n"))
        assertFalse(r.text.contains("model:"))
    }

    @Test fun theModelNameIsWrittenWhenGiven() {
        val r = ConfigYaml.withBackend("", "https://example.com/v1", "my-model", "CUSTOM_API_KEY") as ConfigYaml.Result.Written
        assertTrue(r.text.contains("      model: \"my-model\"\n"))
    }

    @Test fun otherSettingsAreKeptAndAnOwnBackendsListIsNeverOverwritten() {
        val r = ConfigYaml.withBackend("web_port: 5678", "https://example.com", "", "CUSTOM_API_KEY") as ConfigYaml.Result.Written
        assertTrue(r.text.startsWith("web_port: 5678\n# "))
        assertEquals(ConfigYaml.Result.HasBackends, ConfigYaml.withBackend("backends:\n  primary: mine\n", "https://example.com", "", "CUSTOM_API_KEY"))
    }

    @Test fun goingBackAndChangingTheUrlReplacesTheEarlierBlock() {
        val first = (ConfigYaml.withBackend("web_port: 5678\n", "https://one.example.com", "", "CUSTOM_API_KEY") as ConfigYaml.Result.Written).text
        val second = (ConfigYaml.withBackend(first, "https://two.example.com", "", "CUSTOM_API_KEY") as ConfigYaml.Result.Written).text
        assertTrue(second.contains("two.example.com"))
        assertFalse(second.contains("one.example.com"))
        assertEquals(1, Regex("(?m)^backends:").findAll(second).count())
    }
}

class RestoreLogicTest {
    private val secret = "correct horse battery staple"

    @Test fun thePassphraseIsNeverOnTheCommandLine() {
        val args = RestoreLogic.args("/lib/libhq.so", "/cache/restore/backup.hqbak", "/files/hqhome/.vault-restoring", "/cache/restore/secret", secretIsKeyFile = false)
        assertEquals(listOf("/lib/libhq.so", "restore", "/cache/restore/backup.hqbak", "--into", "/files/hqhome/.vault-restoring", "--passphrase-file", "/cache/restore/secret"), args)
        assertFalse(args.any { it.contains(secret) })
        assertFalse(args.contains("--passphrase"))
        assertFalse(args.contains("--passphrase-env"))
    }

    @Test fun aKeyFileIsPassedByPathToo() {
        val args = RestoreLogic.args("/lib/libhq.so", "/a", "/b", "/c", secretIsKeyFile = true)
        assertEquals(listOf("--key-file", "/c"), args.takeLast(2))
    }

    @Test fun theChildSeesOnlyWhatItNeeds() {
        val env = RestoreLogic.env("/h", "/t", "/h/config.yaml", "/system", null)
        assertEquals(setOf("HOME", "TMPDIR", "HQ_CONFIG_PATH", "ANDROID_ROOT"), env.keys)
    }

    @Test fun aPassphraseFileHoldsTheTextOnOneLineAndIsCheckedForLength() {
        assertEquals("$secret\n", RestoreLogic.passphraseFileText("  $secret  "))
        assertNull(RestoreLogic.passphraseProblem(secret))
        assertEquals(FirstRunWords.ERR_PASSPHRASE_SHORT, RestoreLogic.passphraseProblem("too short"))
        assertEquals(FirstRunWords.ERR_PASSPHRASE_SHORT, RestoreLogic.passphraseProblem("123456789"))
    }

    @Test fun aFailureReasonIsTheLastLineWithTokensBlanked() {
        val reason = RestoreLogic.reason(listOf("reading", "", "wrong passphrase " + "a".repeat(40)))
        assertTrue(reason.startsWith("wrong passphrase "))
        assertFalse(reason.contains("a".repeat(40)))
        assertEquals("", RestoreLogic.reason(emptyList()))
    }
}
