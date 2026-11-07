package io.github.akirosetta.tacknote

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class WidgetArgs {
  var noteId: String = ""
}

@InvokeArg
class PasswordArgs {
  var password: String = ""
}

/** What the Rust side asks of Android. Registered in `src-tauri/src/android.rs`. */
@TauriPlugin
class BridgePlugin(private val activity: Activity) : Plugin(activity) {
  /** The note on the widget changed: redraw every widget. */
  @Command
  fun refreshWidget(invoke: Invoke) {
    NoteWidget.refresh(activity)
    invoke.resolve()
  }

  /** Asks the launcher to add a widget showing this note; answers whether it could ask. */
  @Command
  fun addWidget(invoke: Invoke) {
    val noteId = invoke.parseArgs(WidgetArgs::class.java).noteId
    NoteWidget.refresh(activity)
    invoke.resolve(JSObject().put("pinned", NoteWidget.requestPin(activity, noteId)))
  }

  /** The saved WebDAV password, or no `password` field when none is saved. */
  @Command
  fun getPassword(invoke: Invoke) {
    try {
      val answer = JSObject()
      Secrets.read(activity)?.let { answer.put("password", it) }
      invoke.resolve(answer)
    } catch (error: Exception) {
      invoke.reject("the password could not be read: ${error.javaClass.simpleName}")
    }
  }

  @Command
  fun setPassword(invoke: Invoke) {
    try {
      Secrets.write(activity, invoke.parseArgs(PasswordArgs::class.java).password)
      invoke.resolve()
    } catch (error: Exception) {
      invoke.reject("the password could not be saved: ${error.javaClass.simpleName}")
    }
  }
}
