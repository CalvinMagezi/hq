package example.hq.spike.ui.shell

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

/** The design's 24 x 24 outline icons, drawn from the same SVG path data (stroke 2, round ends). */
private fun outline(name: String, vararg paths: String): ImageVector {
    val b = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f)
    for (d in paths) {
        b.addPath(
            pathData = PathParser().parsePathString(d).toNodes(),
            stroke = SolidColor(Color.Black), strokeLineWidth = 2f,
            strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round,
        )
    }
    return b.build()
}

object HqIcons {
    val Chat = outline("chat", "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z")
    val ChevronDown = outline("chevron-down", "m6 9 6 6 6-6")
    val ChevronUp = outline("chevron-up", "m18 15-6-6-6 6")
    val Send = outline(
        "send", "M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z",
        "m21.854 2.147-10.94 10.939",
    )
    val Spark = outline("spark", "M12 3l1.9 4.7L19 9.6l-5.1 1.9L12 16l-1.9-4.5L5 9.6l5.1-1.9z")
    val BarChart = outline("bar-chart", "M18 20V10", "M12 20V4", "M6 20v-6")
    val Expand = outline("expand", "m15 3 6 0 0 6", "M21 3l-7 7", "M9 21H3v-6", "M3 21l7-7")
    val Copy = outline(
        "copy", "M10 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2z",
        "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2",
    )
    val Code = outline("code", "m16 18 6-6-6-6", "m8 6-6 6 6 6")
    val Image = outline(
        "image", "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M9 8a1 1 0 1 0 0 2a1 1 0 1 0 0-2",
        "m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21",
    )
    val External = outline("external", "M15 3h6v6", "M10 14 21 3", "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6")
    val ChevronRight = outline("chevron-right", "m9 18 6-6-6-6")
    val ArrowBack = outline("arrow-back", "m12 19-7-7 7-7", "M19 12H5")
    val MoreVertical = outline("more-vertical", "M12 11a1 1 0 1 0 0 2a1 1 0 1 0 0-2", "M12 4a1 1 0 1 0 0 2a1 1 0 1 0 0-2", "M12 18a1 1 0 1 0 0 2a1 1 0 1 0 0-2")
    val Table = outline("table", "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M3 9h18", "M3 15h18", "M9 3v18")
    val CheckSmall = outline("check-small", "M20 6 9 17l-5-5")
    val File = outline("file", "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z", "M14 2v4a2 2 0 0 0 2 2h4")
    val Camera = outline(
        "camera", "M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3z",
        "M12 10a3 3 0 1 0 0 6a3 3 0 1 0 0-6",
    )
    val Info = outline("info", "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z", "M12 16v-4", "M12 8h.01")
    val Pause = outline("pause", "M10 4H6v16h4z", "M18 4h-4v16h4z")
    val Eye = outline("eye", "M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z", "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z")
    val EyeOff = outline("eye-off", "M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94", "M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19", "m1 1 22 22", "M14.12 14.12a3 3 0 1 1-4.24-4.24")
    val Folder = outline("folder", "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z")
    val Sort = outline("sort", "m21 16-4 4-4-4", "M17 20V4", "m3 8 4-4 4 4", "M7 4v16")
    val List = outline("list", "M8 6h13", "M8 12h13", "M8 18h13", "M3 6h.01", "M3 12h.01", "M3 18h.01")
    val Link = outline("link", "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71", "M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71")
    val Pin = outline(
        "pin", "M12 17v5",
        "M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z",
    )
    val Share = outline(
        "share", "M18 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6z", "M6 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z", "M18 22a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
        "m8.59 13.51 6.83 3.98", "m15.41 6.51-6.82 3.98",
    )
    val Download = outline("download", "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4", "m7 10 5 5 5-5", "M12 15V3")
    val RotateCw = outline("rotate-cw", "M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8", "M21 3v5h-5")
    val Edit = outline("edit", "M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z")
    val TextCursor = outline("text-cursor", "M8 4h8", "M12 4v16", "M8 20h8")
    val Close = outline("close", "M18 6 6 18", "m6 6 12 12")
    val Check = outline("check", "M20 6 9 17l-5-5")
    val Plus = outline("plus", "M5 12h14", "M12 5v14")
    val Warning = outline(
        "warning", "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3", "M12 9v4", "M12 17h.01",
    )
    val WifiOff = outline(
        "wifi-off", "M12 20h.01", "M8.5 16.429a5 5 0 0 1 7 0", "M5 12.859a10 10 0 0 1 5.17-2.69", "M19 12.859a10 10 0 0 0-2.007-1.523",
        "M2 8.82a15 15 0 0 1 4.177-2.643", "M22 8.82a15 15 0 0 0-11.288-3.764", "m2 2 20 20",
    )
    val Redo = outline("redo", "M21 7v6h-6", "M3 17a9 9 0 0 1 9-9 9 9 0 0 1 6 2.3l3 2.7")
    val Archive = outline(
        "archive", "M3 3h18a1 1 0 0 1 1 1v3a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z", "M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8", "M10 12h4",
    )
    val Notes = outline(
        "notes", "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z", "M14 2v4a2 2 0 0 0 2 2h4", "M8 13h8", "M8 17h5",
    )
    val Tasks = outline("tasks", "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "m9 12 2 2 4-4")
    val Helpers = outline(
        "helpers", "M9.94 14.06 4 20", "M12 3l1.9 4.7L19 9.6l-5.1 1.9L12 16l-1.9-4.5L5 9.6l5.1-1.9z",
        "M19 15l.8 2 2.2.8-2.2.8-.8 2.2-.8-2.2L16 17.8l2.2-.8z",
    )
    val Inbox = outline("inbox", "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9", "M10.3 21a1.94 1.94 0 0 0 3.4 0")
    val Search = outline("search", "M3 11a8 8 0 1 0 16 0a8 8 0 1 0-16 0", "m21 21-4.3-4.3")
    val Gear = outline(
        "gear", "M9 12a3 3 0 1 0 6 0a3 3 0 1 0-6 0",
        "M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h0a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h0a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
    )
    val Power = outline("power", "M12 2v10", "M18.4 6.6a9 9 0 1 1-12.77.04")
    val Battery = outline("battery", "M4 7h12a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2z", "M22 11v2")
    val Key = outline("key", "m21 2-9.6 9.6", "M2 15.5a5.5 5.5 0 1 0 11 0a5.5 5.5 0 1 0-11 0", "m15.5 7.5 3 3L22 7l-3-3")
    val Plug = outline("plug", "M12 22v-5", "M9 8V2", "M15 8V2", "M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8z")
    val Phone = outline("phone", "M7 2h10a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z", "M12 18h.01")
    val Shield = outline("shield", "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z")
    val Gauge = outline("gauge", "m12 14 4-4", "M3.34 19a10 10 0 1 1 17.32 0")
    val Lock = outline("lock", "M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z", "M7 11V7a5 5 0 0 1 10 0v4")
}
