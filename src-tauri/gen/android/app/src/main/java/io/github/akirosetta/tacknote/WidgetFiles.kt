package io.github.akirosetta.tacknote

import android.content.Context
import org.json.JSONObject
import java.io.File

class WidgetLine(val text: String, val checked: Boolean?)

class WidgetNote(val id: String, val color: Int, val lines: List<WidgetLine>) {
  /** The first line with text, for lists. */
  val title: String get() = lines.firstOrNull { it.text.isNotBlank() }?.text ?: ""
}

class WidgetFeed(val chosen: String?, val notes: List<WidgetNote>)

/**
 * The files shared with the Rust side, in Tauri's app data folder, which on
 * Android is the app's own data folder. Rust writes `widget.json`
 * (`src-tauri/src/widget.rs`); this side only reads it, and leaves to-dos
 * ticked on a widget in `widget-actions/` for Rust to apply.
 */
object WidgetFiles {
  private fun feedFile(context: Context) = File(context.dataDir, "widget.json")
  private fun actionsDir(context: Context) = File(context.dataDir, "widget-actions")

  fun read(context: Context): WidgetFeed = try {
    val json = JSONObject(feedFile(context).readText())
    val list = json.getJSONArray("notes")
    val notes = (0 until list.length()).map { index ->
      val note = list.getJSONObject(index)
      val lines = note.getJSONArray("lines")
      WidgetNote(
        note.getString("id"),
        note.optInt("colorIndex"),
        (0 until lines.length()).map { at ->
          val line = lines.getJSONObject(at)
          WidgetLine(line.optString("text"), if (line.isNull("checked")) null else line.getBoolean("checked"))
        },
      )
    }
    WidgetFeed(if (json.isNull("chosen")) null else json.optString("chosen"), notes)
  } catch (error: Exception) {
    WidgetFeed(null, emptyList())
  }

  /** A to-do ticked on a widget, waiting for Rust to apply it. */
  fun queueTick(context: Context, noteId: String, line: Int, text: String, checked: Boolean) {
    val dir = actionsDir(context).apply { mkdirs() }
    val json = JSONObject().put("noteId", noteId).put("line", line).put("text", text).put("checked", checked)
    // Written aside first, then renamed, so Rust never reads half a file.
    val name = "${System.currentTimeMillis()}-${System.nanoTime()}.json"
    val temporary = File(dir, "$name.tmp")
    temporary.writeText(json.toString())
    temporary.renameTo(File(dir, name))
  }

  /** The ticks Rust has not applied yet, shown on the widget meanwhile. */
  fun pendingTicks(context: Context, noteId: String): List<JSONObject> =
    actionsDir(context).listFiles { file -> file.name.endsWith(".json") }
      ?.sortedBy { it.name }
      ?.mapNotNull { file -> try { JSONObject(file.readText()) } catch (error: Exception) { null } }
      ?.filter { it.optString("noteId") == noteId }
      ?: emptyList()
}

/** Which note each widget shows, kept on this phone only. */
object WidgetChoices {
  private fun preferences(context: Context) = context.getSharedPreferences("tacknote-widgets", Context.MODE_PRIVATE)

  fun noteFor(context: Context, widgetId: Int): String? = preferences(context).getString("note-$widgetId", null)

  fun choose(context: Context, widgetId: Int, noteId: String) {
    preferences(context).edit().putString("note-$widgetId", noteId).apply()
  }

  fun forget(context: Context, widgetIds: IntArray) {
    val editor = preferences(context).edit()
    for (id in widgetIds) editor.remove("note-$id")
    editor.apply()
  }
}
