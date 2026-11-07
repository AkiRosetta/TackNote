package io.github.akirosetta.tacknote

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

/** What the Rust side asks of Android. Registered in `src-tauri/src/android.rs`. */
@TauriPlugin
class BridgePlugin(private val activity: Activity) : Plugin(activity) {
  /** The note on the widget changed: redraw every widget. */
  @Command
  fun refreshWidget(invoke: Invoke) {
    NoteWidget.refresh(activity)
    invoke.resolve()
  }

  /** A note was chosen for the widget: redraw, and offer to add a widget if there is none. */
  @Command
  fun showWidget(invoke: Invoke) {
    NoteWidget.refresh(activity)
    if (!NoteWidget.isPlaced(activity)) NoteWidget.requestPin(activity)
    invoke.resolve()
  }
}
