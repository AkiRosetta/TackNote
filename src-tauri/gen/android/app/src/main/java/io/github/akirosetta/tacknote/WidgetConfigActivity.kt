package io.github.akirosetta.tacknote

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import android.util.TypedValue
import android.view.View
import android.view.ViewGroup
import android.widget.AbsListView
import android.widget.BaseAdapter
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.TextView

/**
 * Picks the note a widget shows: opened by the launcher when a widget is
 * added by hand, and by tapping a widget whose note is gone.
 */
class WidgetConfigActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    setResult(RESULT_CANCELED)
    val widgetId = intent?.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
      ?: AppWidgetManager.INVALID_APPWIDGET_ID
    if (widgetId == AppWidgetManager.INVALID_APPWIDGET_ID) {
      finish()
      return
    }
    val notes = WidgetFiles.read(this).notes
    val padding = dp(16)
    val root = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      setPadding(padding, padding, padding, 0)
    }
    // The frame keeps clear of the status and gesture bars; fitting them
    // replaces a view's own padding, hence the two layers.
    val frame = android.widget.FrameLayout(this).apply {
      fitsSystemWindows = true
      addView(root)
    }
    root.addView(TextView(this).apply {
      setText(if (notes.isEmpty()) R.string.widget_no_notes else R.string.widget_choose)
      setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
      setPadding(0, 0, 0, padding)
    })
    val list = ListView(this).apply {
      divider = null
      dividerHeight = dp(10)
      adapter = object : BaseAdapter() {
        override fun getCount() = notes.size
        override fun getItem(position: Int) = notes[position]
        override fun getItemId(position: Int) = position.toLong()
        override fun getView(position: Int, convertView: View?, parent: ViewGroup?): View {
          val note = notes[position]
          return (convertView as? TextView ?: TextView(this@WidgetConfigActivity)).apply {
            text = note.title.ifBlank { getString(R.string.widget_blank) }
            maxLines = 2
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
            setTextColor(getColor(R.color.widget_text))
            setBackgroundResource(NoteWidget.background(note.color))
            setPadding(padding, padding, padding, padding)
            layoutParams = AbsListView.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
          }
        }
      }
      setOnItemClickListener { _, _, position, _ ->
        WidgetChoices.choose(this@WidgetConfigActivity, widgetId, notes[position].id)
        NoteWidget.refresh(this@WidgetConfigActivity)
        setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widgetId))
        finish()
      }
    }
    root.addView(list, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
    setContentView(frame)
  }

  private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
}
