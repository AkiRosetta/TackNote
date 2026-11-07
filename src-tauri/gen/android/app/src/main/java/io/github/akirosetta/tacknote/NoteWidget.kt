package io.github.akirosetta.tacknote

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Paint
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

    private val ROWS = intArrayOf(
      R.id.row_0, R.id.row_1, R.id.row_2, R.id.row_3, R.id.row_4, R.id.row_5,
      R.id.row_6, R.id.row_7, R.id.row_8, R.id.row_9, R.id.row_10, R.id.row_11,
    )
    private val BOXES = intArrayOf(
      R.id.box_0, R.id.box_1, R.id.box_2, R.id.box_3, R.id.box_4, R.id.box_5,
      R.id.box_6, R.id.box_7, R.id.box_8, R.id.box_9, R.id.box_10, R.id.box_11,
    )
    private val TEXTS = intArrayOf(
      R.id.text_0, R.id.text_1, R.id.text_2, R.id.text_3, R.id.text_4, R.id.text_5,
      R.id.text_6, R.id.text_7, R.id.text_8, R.id.text_9, R.id.text_10, R.id.text_11,
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
      val lines = note?.let { shownLines(context, it) } ?: emptyList()
      for ((index, row) in ROWS.withIndex()) {
        val line = lines.getOrNull(index)
        views.setViewVisibility(row, if (line == null) View.GONE else View.VISIBLE)
      }
      if (note == null || lines.all { it.text.isBlank() && it.checked == null }) {
        // No note yet or it was deleted: a tap lets the user pick one. An
        // empty note just says so and opens the app.
        val action = if (note == null) chooseNote(context, widgetId) else openApp(context)
        views.setInt(R.id.widget_root, "setBackgroundResource", background(note?.color ?: 0))
        views.setTextViewText(R.id.widget_hint, context.getString(if (note == null) R.string.widget_empty else R.string.widget_blank))
        views.setViewVisibility(R.id.widget_hint, View.VISIBLE)
        for (row in ROWS) views.setViewVisibility(row, View.GONE)
        views.setOnClickPendingIntent(R.id.widget_root, action)
        views.setOnClickPendingIntent(R.id.widget_hint, action)
        return views
      }
      val open = openApp(context)
      views.setViewVisibility(R.id.widget_hint, View.GONE)
      views.setInt(R.id.widget_root, "setBackgroundResource", background(note.color))
      views.setOnClickPendingIntent(R.id.widget_root, open)
      for ((index, line) in lines.take(ROWS.size).withIndex()) {
        views.setTextViewText(TEXTS[index], line.text.ifEmpty { " " })
        views.setOnClickPendingIntent(TEXTS[index], open)
        // A ticked to-do is struck through, as in the app.
        val flags = Paint.ANTI_ALIAS_FLAG or (if (line.checked == true) Paint.STRIKE_THRU_TEXT_FLAG else 0)
        views.setInt(TEXTS[index], "setPaintFlags", flags)
        if (line.checked == null) {
          views.setViewVisibility(BOXES[index], View.GONE)
        } else {
          views.setViewVisibility(BOXES[index], View.VISIBLE)
          views.setTextViewText(BOXES[index], if (line.checked) "☑" else "☐")
          views.setOnClickPendingIntent(BOXES[index], tick(context, widgetId, note.id, index, line))
        }
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
