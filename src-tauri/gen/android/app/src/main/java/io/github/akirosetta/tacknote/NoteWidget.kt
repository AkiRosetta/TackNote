package io.github.akirosetta.tacknote

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.view.View
import android.widget.RemoteViews

/**
 * A home screen widget showing one note, line by line. Tapping a to-do ticks
 * it; tapping anything else opens the app. Each widget shows its own note,
 * chosen when it is added (see `WidgetConfigActivity` and `addWidget`).
 */
class NoteWidget : AppWidgetProvider() {
  override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
    for (id in ids) manager.updateAppWidget(id, views(context, id))
  }

  override fun onDeleted(context: Context, ids: IntArray) {
    WidgetChoices.forget(context, ids)
  }

  override fun onReceive(context: Context, intent: Intent) {
    when (intent.action) {
      ACTION_TICK -> {
        val noteId = intent.getStringExtra(EXTRA_NOTE) ?: return
        WidgetFiles.queueTick(
          context,
          noteId,
          intent.getIntExtra(EXTRA_LINE, -1),
          intent.getStringExtra(EXTRA_TEXT) ?: "",
          intent.getBooleanExtra(EXTRA_CHECKED, true),
        )
        refresh(context)
      }
      ACTION_PINNED -> {
        val widgetId = intent.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
        val noteId = intent.getStringExtra(EXTRA_NOTE)
        if (widgetId != AppWidgetManager.INVALID_APPWIDGET_ID && noteId != null) {
          WidgetChoices.choose(context, widgetId, noteId)
          refresh(context)
        }
      }
      else -> super.onReceive(context, intent)
    }
  }

  companion object {
    private const val ACTION_TICK = "io.github.akirosetta.tacknote.TICK"
    private const val ACTION_PINNED = "io.github.akirosetta.tacknote.PINNED"
    private const val EXTRA_NOTE = "noteId"
    private const val EXTRA_LINE = "line"
    private const val EXTRA_TEXT = "text"
    private const val EXTRA_CHECKED = "checked"

    /** Drawn by the launcher, so day and night colours follow the system. */
    private val BACKGROUNDS = intArrayOf(
      R.drawable.widget_bg_0, R.drawable.widget_bg_1, R.drawable.widget_bg_2,
      R.drawable.widget_bg_3, R.drawable.widget_bg_4, R.drawable.widget_bg_5,
    )

    fun background(color: Int) = BACKGROUNDS[color.coerceIn(0, BACKGROUNDS.size - 1)]

    private val LINES = intArrayOf(
      R.id.line_0, R.id.line_1, R.id.line_2, R.id.line_3, R.id.line_4, R.id.line_5,
      R.id.line_6, R.id.line_7, R.id.line_8, R.id.line_9, R.id.line_10, R.id.line_11,
    )

    /** The widget's own note, or for widgets from before each had one, the note chosen then. */
    private fun noteFor(context: Context, widgetId: Int, feed: WidgetFeed): WidgetNote? {
      val id = WidgetChoices.noteFor(context, widgetId) ?: feed.chosen ?: return null
      return feed.notes.firstOrNull { it.id == id }
    }

    /** Lines with the ticks Rust has not applied yet already shown. */
    private fun shownLines(context: Context, note: WidgetNote): List<WidgetLine> {
      val lines = note.lines.toMutableList()
      for (tick in WidgetFiles.pendingTicks(context, note.id)) {
        val at = tick.optInt("line", -1)
        if (at in lines.indices && lines[at].checked != null && lines[at].text == tick.optString("text")) {
          lines[at] = WidgetLine(lines[at].text, tick.optBoolean("checked"))
        }
      }
      return lines
    }

    private fun openApp(context: Context) = PendingIntent.getActivity(
      context, 0, Intent(context, MainActivity::class.java),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun chooseNote(context: Context, widgetId: Int) = PendingIntent.getActivity(
      context, widgetId,
      Intent(context, WidgetConfigActivity::class.java).putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widgetId),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun tick(context: Context, widgetId: Int, noteId: String, line: Int, shown: WidgetLine) = PendingIntent.getBroadcast(
      context,
      widgetId * 100 + line,
      Intent(context, NoteWidget::class.java).setAction(ACTION_TICK)
        .putExtra(EXTRA_NOTE, noteId).putExtra(EXTRA_LINE, line)
        .putExtra(EXTRA_TEXT, shown.text).putExtra(EXTRA_CHECKED, shown.checked != true),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun views(context: Context, widgetId: Int, feed: WidgetFeed = WidgetFiles.read(context)): RemoteViews {
      val views = RemoteViews(context.packageName, R.layout.note_widget)
      val note = noteFor(context, widgetId, feed)
      if (note == null) {
        // No note yet, or it was deleted: a tap lets the user pick one.
        views.setInt(R.id.widget_root, "setBackgroundResource", background(0))
        views.setTextViewText(LINES[0], context.getString(R.string.widget_empty))
        views.setViewVisibility(LINES[0], View.VISIBLE)
        for (id in LINES.drop(1)) views.setViewVisibility(id, View.GONE)
        views.setOnClickPendingIntent(R.id.widget_root, chooseNote(context, widgetId))
        views.setOnClickPendingIntent(LINES[0], chooseNote(context, widgetId))
        return views
      }
      val open = openApp(context)
      views.setInt(R.id.widget_root, "setBackgroundResource", background(note.color))
      views.setOnClickPendingIntent(R.id.widget_root, open)
      val lines = shownLines(context, note)
      for ((index, id) in LINES.withIndex()) {
        val line = lines.getOrNull(index)
        if (line == null || (index == 0 && lines.size == 1 && line.text.isBlank())) {
          views.setViewVisibility(id, View.GONE)
          continue
        }
        val mark = when (line.checked) { true -> "☑ "; false -> "☐ "; null -> "" }
        views.setTextViewText(id, mark + line.text.ifEmpty { " " })
        views.setViewVisibility(id, View.VISIBLE)
        views.setOnClickPendingIntent(id, if (line.checked == null) open else tick(context, widgetId, note.id, index, line))
      }
      if (lines.all { it.text.isBlank() }) {
        views.setTextViewText(LINES[0], context.getString(R.string.widget_blank))
        views.setViewVisibility(LINES[0], View.VISIBLE)
        views.setOnClickPendingIntent(LINES[0], open)
      }
      return views
    }

    private fun ids(context: Context): IntArray =
      AppWidgetManager.getInstance(context).getAppWidgetIds(ComponentName(context, NoteWidget::class.java))

    /** Redraws every placed widget from the files. */
    fun refresh(context: Context) {
      val manager = AppWidgetManager.getInstance(context)
      val feed = WidgetFiles.read(context)
      for (id in ids(context)) manager.updateAppWidget(id, views(context, id, feed))
    }

    /**
     * Asks the launcher to add a widget showing this note; the user confirms
     * in its dialog. Says whether it could ask.
     */
    fun requestPin(context: Context, noteId: String): Boolean {
      val manager = AppWidgetManager.getInstance(context)
      if (!manager.isRequestPinAppWidgetSupported) return false
      // Mutable: the launcher adds the new widget's id to it.
      val added = PendingIntent.getBroadcast(
        context,
        noteId.hashCode(),
        Intent(context, NoteWidget::class.java).setAction(ACTION_PINNED).putExtra(EXTRA_NOTE, noteId),
        PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
      )
      return manager.requestPinAppWidget(ComponentName(context, NoteWidget::class.java), null, added)
    }
  }
}
