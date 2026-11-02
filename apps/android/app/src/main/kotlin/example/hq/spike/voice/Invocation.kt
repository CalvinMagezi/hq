package example.hq.spike.voice

/**
 * People open with "Hey HQ" out of habit, and the recogniser (trained on audiobooks) has never heard "HQ": it comes
 * out as "Ue", "h q", "age queue" and the like. This takes that opening off a transcript so only the question is
 * sent. It only ever removes a greeting at the very start, and never leaves an empty question.
 */
object Invocation {
    // Each form was heard on the A32 ("Ue", "Ahq,") or is how the letters H and Q come out of the model's vocabulary.
    private val OPENING = Regex(
        """^\s*(?:(?:hey|hay|hi|ok|okay|a)[\s,]*)?(?:ue|h\s*q|h\s*queue|h\s*cue|h\s*you|aitch\s*(?:q|queue|cue|you)|age\s*(?:q|queue|cue|you)|hq)\b[\s,.!?]*""",
        RegexOption.IGNORE_CASE,
    )

    fun strip(transcript: String): String {
        val rest = OPENING.replaceFirst(transcript, "")
        if (rest.isBlank()) return transcript.trim()
        return rest.trim().replaceFirstChar { it.titlecase() }
    }
}
