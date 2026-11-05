package example.hq.spike.update

/** Whether the app's screen is in front. Android's install confirm is started only then; otherwise a notice carries it. */
object AppVisibility {
    @Volatile var visible: Boolean = false
}
