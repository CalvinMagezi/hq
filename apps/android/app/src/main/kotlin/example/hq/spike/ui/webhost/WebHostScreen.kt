package example.hq.spike.ui.webhost

import android.annotation.SuppressLint
import android.app.AlertDialog
import android.content.ActivityNotFoundException
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import example.hq.spike.HubService
import example.hq.spike.data.HqServices
import kotlinx.coroutines.launch
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val PROGRESS_HEIGHT = 4.dp
private const val FULL = 100

/**
 * A page of the web app (Home, Usage, Board, Timeline) under the app's own bar. Only the local hq loads here, signed in without
 * the token ever being in an address; any other link opens in the browser, and Back walks the page's history before it closes this screen.
 */
@SuppressLint("SetJavaScriptEnabled")
@Composable
fun WebHostScreen(page: WebPage, onClose: () -> Unit) {
    val context = LocalContext.current
    var progress by remember { mutableIntStateOf(0) }
    var webTitle by remember { mutableStateOf<String?>(null) }
    var failed by remember { mutableStateOf(false) }
    var canBack by remember { mutableStateOf(false) }
    val state = remember { HostState() }
    val chooser = remember { arrayOfNulls<ValueCallback<Array<Uri>>>(1) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris ->
        chooser[0]?.onReceiveValue(uris.toTypedArray())
        chooser[0] = null
    }
    val web = remember {
        WebView(context).apply {
            setBackgroundColor(android.graphics.Color.BLACK)
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.setSupportZoom(false)
            settings.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
        }
    }

    val client = remember { HqServices.get(context.applicationContext).client }
    val scope = rememberCoroutineScope()

    /**
     * Signs the page in, then loads it: the token is written into the local hq's own storage from a blank page of that origin.
     * HQ is asked first, because the web app's offline cache would otherwise show an old page and hide that HQ is not answering.
     */
    fun start() {
        failed = false
        progress = 0
        scope.launch {
            if (!client.healthy()) { failed = true; return@launch }
            val token = try { HubService.token(context) } catch (e: IllegalStateException) { failed = true; return@launch }
            state.script = WebHost.tokenScript(token)
            state.hopping = true
            web.loadDataWithBaseURL(WebHost.ORIGIN + "/", "<html></html>", "text/html", "utf-8", null)
        }
    }

    web.webViewClient = object : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            val url = request.url.toString()
            if (WebHost.allows(url)) return false
            if (WebHost.opensInBrowser(url)) {
                try { context.startActivity(Intent(Intent.ACTION_VIEW, request.url).addCategory(Intent.CATEGORY_BROWSABLE)) } catch (e: ActivityNotFoundException) { }
            }
            return true
        }

        override fun onPageStarted(view: WebView, url: String?, favicon: Bitmap?) {
            if (!state.hopping) failed = false
        }

        override fun onPageFinished(view: WebView, url: String?) {
            if (state.hopping) {
                state.hopping = false
                val script = state.script.also { state.script = "" }
                view.evaluateJavascript(script) { state.clearHistory = true; view.loadUrl(WebHost.url(page)) }
                return
            }
            if (state.clearHistory) { state.clearHistory = false; view.clearHistory() }
            canBack = view.canGoBack()
            progress = FULL
        }

        override fun doUpdateVisitedHistory(view: WebView, url: String?, isReload: Boolean) { canBack = view.canGoBack() }

        override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
            if (request.isForMainFrame && !state.hopping) failed = true
        }
    }
    web.webChromeClient = object : WebChromeClient() {
        override fun onProgressChanged(view: WebView, newProgress: Int) { if (!state.hopping) progress = newProgress }
        override fun onReceivedTitle(view: WebView, title: String?) { webTitle = title }

        override fun onJsAlert(view: WebView, url: String?, message: String?, result: JsResult): Boolean {
            AlertDialog.Builder(context).setMessage(message).setPositiveButton(android.R.string.ok) { _, _ -> result.confirm() }.setOnCancelListener { result.cancel() }.show()
            return true
        }

        override fun onJsConfirm(view: WebView, url: String?, message: String?, result: JsResult): Boolean {
            AlertDialog.Builder(context).setMessage(message).setPositiveButton(android.R.string.ok) { _, _ -> result.confirm() }
                .setNegativeButton(android.R.string.cancel) { _, _ -> result.cancel() }.setOnCancelListener { result.cancel() }.show()
            return true
        }

        // A page asking for text is not something HQ's pages do; it is answered with a cancel rather than shown.
        override fun onJsPrompt(view: WebView, url: String?, message: String?, defaultValue: String?, result: JsPromptResult): Boolean { result.cancel(); return true }

        override fun onShowFileChooser(view: WebView, callback: ValueCallback<Array<Uri>>, params: FileChooserParams): Boolean {
            chooser[0]?.onReceiveValue(null)
            chooser[0] = callback
            val type = params.acceptTypes.firstOrNull { it.isNotBlank() } ?: "*/*"
            return try { picker.launch(type); true } catch (e: ActivityNotFoundException) { chooser[0] = null; callback.onReceiveValue(null); false }
        }
    }

    LaunchedEffect(page) { start() }
    DisposableEffect(Unit) { onDispose { chooser[0]?.onReceiveValue(null); web.stopLoading(); web.destroy() } }
    BackHandler(onBack = onClose)
    BackHandler(enabled = canBack && !failed) { web.goBack() }

    Column(Modifier.fillMaxSize().background(Mts.Surface000).consumeTouches().statusBarsPadding().navigationBarsPadding().imePadding()) {
        Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp) { if (canBack && !failed) web.goBack() else onClose() }
            Text(WebHost.barTitle(page, if (failed) null else webTitle), style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f).padding(start = 4.dp))
            IconAction(HqIcons.RotateCw, "Reload", iconSize = 24.dp) { start() }
        }
        Box(Modifier.fillMaxWidth().height(PROGRESS_HEIGHT).background(if (progress in 1 until FULL) Mts.Surface100 else Mts.Surface000)) {
            if (progress in 1 until FULL) Box(Modifier.fillMaxWidth(progress / FULL.toFloat()).fillMaxHeight().background(Mts.BrandBlue))
        }
        Box(Modifier.weight(1f).fillMaxWidth().background(Mts.Surface000)) {
            AndroidView(factory = { web }, modifier = Modifier.fillMaxSize())
            if (failed) FailedPage(page, onRetry = { start() })
        }
    }
}

/** Where the sign-in hop has got to: the blank page is loading, or the real one is. */
private class HostState {
    var hopping = false
    var script = ""
    var clearHistory = false
}

@Composable
private fun FailedPage(page: WebPage, onRetry: () -> Unit) {
    Column(Modifier.fillMaxSize().background(Mts.Surface000).padding(32.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(WebHost.failedTitle(page), style = HqType.H2, color = Mts.Ink, textAlign = TextAlign.Center)
        Text(WebHost.FAILED_BODY, style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center)
        Box(Modifier.padding(top = 8.dp).heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = onRetry).padding(horizontal = 32.dp), contentAlignment = Alignment.Center) {
            Text(WebHost.TRY_AGAIN, style = HqType.BodySmBold, color = Mts.OnGreen)
        }
    }
}
