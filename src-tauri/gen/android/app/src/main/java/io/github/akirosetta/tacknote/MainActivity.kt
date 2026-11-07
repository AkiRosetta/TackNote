package io.github.akirosetta.tacknote

import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  private var keyboardShown = -1

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    watchKeyboard()
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
  }

  /**
   * Drawn edge to edge, the page does not shrink when the keyboard opens and
   * cannot tell how much it covers. Tell it, in CSS pixels, so it can keep
   * the bottom of the page (the colour row) above the keyboard.
   *
   * Listens on the window's content view, which holds the WebView: a listener
   * on the WebView itself replaces its own handling and the page loses its
   * safe areas. The insets are passed on untouched.
   */
  private fun watchKeyboard() {
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
      val css = Math.round(keyboard / view.resources.displayMetrics.density)
      if (css != keyboardShown) {
        keyboardShown = css
        webView?.evaluateJavascript("document.documentElement.style.setProperty('--keyboard','${css}px')", null)
      }
      insets
    }
  }

  /** Leaving the app is when the home screen shows, so redraw then as well. */
  override fun onPause() {
    super.onPause()
    NoteWidget.refresh(this)
  }
}
