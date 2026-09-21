package org.handover.android

import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.view.MotionEvent
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

internal fun MainActivity.buildPresentationPage(): View {
    val activity = this
    var pointerX = 0f
    var pointerY = 0f
    var pointerMoved = false
    val pointerPad = View(this).apply {
        minimumHeight = dp(220)
        background = GradientDrawable().apply {
            setColor(Color.rgb(225, 229, 239))
            cornerRadius = dp(14).toFloat()
        }
        setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    pointerX = event.x
                    pointerY = event.y
                    pointerMoved = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = ((event.x - pointerX) / 2f).toInt()
                    val dy = ((event.y - pointerY) / 2f).toInt()
                    if (dx != 0 || dy != 0) {
                        pointerMoved = true
                        HandoverForegroundService.presentation("pointer_move", dx, dy)
                        pointerX = event.x
                        pointerY = event.y
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (!pointerMoved) {
                        HandoverForegroundService.presentation("pointer_click")
                    }
                    true
                }
                else -> true
            }
        }
    }
    return featurePage(
        "Presentation remote", "Control the active presentation on Linux.",
        panel(sectionTitle("Slides"), LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(secondary(Button(activity).apply {
                text = "Previous"
                setOnClickListener { HandoverForegroundService.presentation("previous") }
            }), LinearLayout.LayoutParams(0, -2, 1f))
            addView(secondary(Button(activity).apply {
                text = "Next"
                setOnClickListener { HandoverForegroundService.presentation("next") }
            }), LinearLayout.LayoutParams(0, -2, 1f))
        }, LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(secondary(Button(activity).apply {
                text = "Start"
                setOnClickListener { HandoverForegroundService.presentation("start") }
            }), LinearLayout.LayoutParams(0, -2, 1f))
            addView(secondary(Button(activity).apply {
                text = "Stop"
                setOnClickListener { HandoverForegroundService.presentation("stop") }
            }), LinearLayout.LayoutParams(0, -2, 1f))
            addView(secondary(Button(activity).apply {
                text = "Fullscreen"
                setOnClickListener { HandoverForegroundService.presentation("fullscreen") }
            }), LinearLayout.LayoutParams(0, -2, 1f))
        }),
        panel(sectionTitle("Pointer"), TextView(this).apply {
            text = "Drag to move. Tap to click."
            setTextColor(Color.rgb(70, 77, 94))
        }, pointerPad),
    )
}

internal fun MainActivity.buildRemoteInputPage(): View {
    val activity = this
    var remoteX = 0f
    var remoteY = 0f
    var remoteMoved = false
    val remotePad = View(this).apply {
        minimumHeight = dp(240)
        background = GradientDrawable().apply {
            setColor(Color.rgb(225, 229, 239))
            cornerRadius = dp(14).toFloat()
        }
        setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    remoteX = event.x
                    remoteY = event.y
                    remoteMoved = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = ((event.x - remoteX) / 2f).toInt()
                    val dy = ((event.y - remoteY) / 2f).toInt()
                    if (dx != 0 || dy != 0) {
                        remoteMoved = true
                        HandoverForegroundService.remoteInput("move", dx, dy)
                        remoteX = event.x
                        remoteY = event.y
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (!remoteMoved) HandoverForegroundService.remoteInput("click", button = 1)
                    true
                }
                else -> true
            }
        }
    }
    val remoteText = EditText(this).apply {
        hint = "Text to type on Linux"
        setSingleLine(false)
    }
    return featurePage(
        "Remote input", "Control the Linux pointer and type text.",
        panel(sectionTitle("Touchpad"), TextView(this).apply {
            text = "Drag to move. Tap to click."
            setTextColor(Color.rgb(70, 77, 94))
        }, remotePad, LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(secondary(Button(activity).apply {
                text = "Left click"
                setOnClickListener { HandoverForegroundService.remoteInput("click", button = 1) }
            }), LinearLayout.LayoutParams(0, -2, 1f))
            addView(secondary(Button(activity).apply {
                text = "Right click"
                setOnClickListener { HandoverForegroundService.remoteInput("click", button = 3) }
            }), LinearLayout.LayoutParams(0, -2, 1f))
        }, LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(secondary(Button(activity).apply {
                text = "Scroll up"
                setOnClickListener { HandoverForegroundService.remoteInput("scroll", deltaY = -1) }
            }), LinearLayout.LayoutParams(0, -2, 1f))
            addView(secondary(Button(activity).apply {
                text = "Scroll down"
                setOnClickListener { HandoverForegroundService.remoteInput("scroll", deltaY = 1) }
            }), LinearLayout.LayoutParams(0, -2, 1f))
        }),
        panel(sectionTitle("Keyboard"), remoteText, secondary(Button(this).apply {
            text = "Type on Linux"
            setOnClickListener {
                HandoverForegroundService.remoteInput("type", text = remoteText.text.toString())
                remoteText.text.clear()
            }
        })),
    )
}

internal fun MainActivity.buildRemoteDesktopPage(): View {
    val activity = this
    val browsePath = EditText(this).apply {
        hint = "Path relative to Linux home, or ."
        setSingleLine(true)
        setText(".")
    }
    val commandName = EditText(this).apply {
        hint = "Allowlisted command name"
        setSingleLine(true)
    }
    remoteResult = TextView(this).apply {
        text = "Results appear here after the daemon replies."
        setTextColor(Color.rgb(70, 77, 94))
    }
    return featurePage(
        "Remote desktop", "Browse the Linux home directory or run a configured command.",
        panel(
            sectionTitle("Browse Linux files"),
            browsePath,
            LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                addView(secondary(Button(activity).apply {
                    text = "List"
                    setOnClickListener {
                        if (!HandoverForegroundService.browseLinux(browsePath.text.toString()))
                            remoteResult.text = "Directory request was not accepted"
                    }
                }), LinearLayout.LayoutParams(0, -2, 1f))
                addView(secondary(Button(activity).apply {
                    text = "Up"
                    setOnClickListener {
                        val current = browsePath.text.toString()
                        browsePath.setText(
                            current.substringBeforeLast('/', missingDelimiterValue = ".")
                                .ifEmpty { "." },
                        )
                    }
                }), LinearLayout.LayoutParams(0, -2, 1f))
            },
            remoteResult,
        ),
        panel(
            sectionTitle("Configured commands"),
            commandName,
            secondary(Button(this).apply {
                text = "List commands"
                setOnClickListener {
                    if (!HandoverForegroundService.requestLinuxCommands())
                        remoteResult.text = "Command list request was not accepted"
                }
            }),
            secondary(Button(this).apply {
                text = "Run command"
                setOnClickListener {
                    if (!HandoverForegroundService.runLinuxCommand(commandName.text.toString()))
                        remoteResult.text = "Remote command request was not accepted"
                }
            }),
        ),
    )
}
