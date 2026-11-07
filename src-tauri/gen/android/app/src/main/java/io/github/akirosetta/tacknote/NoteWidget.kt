package io.github.akirosetta.tacknote

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews
import org.json.JSONObject
import java.io.File

/**
 * Shows the note chosen in the app. The Rust side writes `widget.json` next
 * to the notes (`src-tauri/src/widget.rs`); this only reads it, so the widget
 * can draw itself while the app is closed.
 */
class NoteWidget : AppWidgetProvider() {
  override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
    for (id in ids) manager.updateAppWidget(id, views(context))
  }

  private class Shown(val text: String, val color: Int)

  companion object {
    /** Where Tauri's app data folder is on Android: the app's own data folder. */
    private fun file(context: Context) = File(context.dataDir, "widget.json")

    /** Drawn by the launcher, so day and night colours follow the system. */
    private val BACKGROUNDS = intArrayOf(
      R.drawable.widget_bg_0, R.drawable.widget_bg_1, R.drawable.widget_bg_2,
      R.drawable.widget_bg_3, R.drawable.widget_bg_4, R.drawable.widget_bg_5,
    )

    /** Null when no note is chosen yet, or the file cannot be read. */
    private fun read(context: Context): Shown? = try {
      val json = JSONObject(file(context).readText())
      if (json.isNull("noteId")) null else Shown(json.optString("text"), json.optInt("colorIndex"))
    } catch (error: Exception) {
      null
    }

    private fun views(context: Context): RemoteViews {
      val shown = read(context)
      val text = when {
        shown == null -> context.getString(R.string.widget_empty)
        shown.text.isBlank() -> context.getString(R.string.widget_blank)
        else -> shown.text
      }
      val open = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
      )
      return RemoteViews(context.packageName, R.layout.note_widget).apply {
        setTextViewText(R.id.widget_text, text)
        setInt(R.id.widget_root, "setBackgroundResource", BACKGROUNDS[(shown?.color ?: 0).coerceIn(0, BACKGROUNDS.size - 1)])
        setOnClickPendingIntent(R.id.widget_root, open)
      }
    }

    private fun ids(context: Context): IntArray =
      AppWidgetManager.getInstance(context).getAppWidgetIds(ComponentName(context, NoteWidget::class.java))

    /** Redraws every placed widget from the file. */
    fun refresh(context: Context) {
      val manager = AppWidgetManager.getInstance(context)
      for (id in ids(context)) manager.updateAppWidget(id, views(context))
    }

    fun isPlaced(context: Context) = ids(context).isNotEmpty()

    /** Asks the launcher to add the widget; the user confirms in its dialog. */
    fun requestPin(context: Context) {
      val manager = AppWidgetManager.getInstance(context)
      if (manager.isRequestPinAppWidgetSupported) {
        manager.requestPinAppWidget(ComponentName(context, NoteWidget::class.java), null, null)
      }
    }
  }
}
