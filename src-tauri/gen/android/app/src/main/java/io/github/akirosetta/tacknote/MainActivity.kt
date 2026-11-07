package io.github.akirosetta.tacknote

import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  /** Leaving the app is when the home screen shows, so redraw then as well. */
  override fun onPause() {
    super.onPause()
    NoteWidget.refresh(this)
  }
}
