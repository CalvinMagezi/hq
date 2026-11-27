package example.hq.spike.ui.reminders

import example.hq.spike.data.Reminder
import example.hq.spike.ui.reminders.RemindersLogic.Choice
import example.hq.spike.ui.reminders.RemindersLogic.Group
import example.hq.spike.ui.reminders.RemindersLogic.RepeatChoice
import java.time.ZoneId
import java.time.ZonedDateTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class RemindersLogicTest {
    private val kampala = ZoneId.of("Africa/Kampala")
    private val london = ZoneId.of("Europe/London")

    // Friday 27 November 2026, 13:20 in Kampala.
    private val now = ZonedDateTime.of(2026, 11, 27, 13, 20, 0, 0, kampala)

    private fun at(day: Int, hour: Int, minute: Int = 0, zone: ZoneId = kampala) = ZonedDateTime.of(2026, 11, day, hour, minute, 0, 0, zone)

    private fun reminder(id: String, due: ZonedDateTime) =
        Reminder(id, id, null, "active", "", due.toEpochSecond(), "Africa/Kampala", null, false, 0, false)

    @Test fun theQuickTimesAreAnHourEveningAndTomorrowMorningInTheOwnersZone() {
        val options = RemindersLogic.options(now)
        assertEquals(listOf(Choice.Hour, Choice.Evening, Choice.Tomorrow), options.map { it.choice })
        assertEquals(at(27, 14, 20), options[0].at)
        assertEquals(at(27, 18), options[1].at)
        assertEquals(at(28, 9), options[2].at)
    }

    @Test fun thisEveningIsLeftOutWhenItIsNearlyNowOrPast() {
        assertEquals(listOf(Choice.Hour, Choice.Tomorrow), RemindersLogic.options(at(27, 17, 45)).map { it.choice })
        assertEquals(listOf(Choice.Hour, Choice.Tomorrow), RemindersLogic.options(at(27, 22)).map { it.choice })
    }

    @Test fun nextMondayIsTheComingMondayNeverToday() {
        assertEquals(at(30, 9), RemindersLogic.nextMonday(now))
        assertEquals(ZonedDateTime.of(2026, 12, 7, 9, 0, 0, 0, kampala), RemindersLogic.nextMonday(at(30, 8)))
        assertEquals(Choice.Monday, RemindersLogic.options(now, monday = true).last().choice)
    }

    @Test fun aTimeTheClocksSkipRingsAfterTheGapAndARepeatedOneIsTheFirst() {
        // London springs forward at 01:00 on 29 March 2026: 01:30 does not exist.
        val skipped = ZonedDateTime.of(2026, 3, 29, 1, 30, 0, 0, london)
        assertEquals(java.time.Instant.parse("2026-03-29T01:30:00Z"), skipped.toInstant())
        // It falls back at 02:00 on 25 October 2026: 01:30 happens twice, and the first is summer time.
        val twice = ZonedDateTime.of(2026, 10, 25, 1, 30, 0, 0, london)
        assertEquals(java.time.Instant.parse("2026-10-25T00:30:00Z"), twice.toInstant())
    }

    @Test fun tomorrowMorningKeepsItsWallClockAcrossAClockChange() {
        val saturday = ZonedDateTime.of(2026, 3, 28, 12, 0, 0, 0, london)
        val next = RemindersLogic.tomorrowMorning(saturday)
        assertEquals(9, next.hour)
        assertEquals(29, next.dayOfMonth)
    }

    @Test fun weeklyAndMonthlyFollowTheDayItFirstRings() {
        assertEquals("weekly:5", RemindersLogic.repeatSpec(RepeatChoice.Weekly, at(27, 16)))
        assertEquals("monthly:27", RemindersLogic.repeatSpec(RepeatChoice.Monthly, at(27, 16)))
        assertEquals("daily", RemindersLogic.repeatSpec(RepeatChoice.Daily, at(27, 16)))
        assertEquals("weekdays", RemindersLogic.repeatSpec(RepeatChoice.Weekdays, at(27, 16)))
        assertEquals(null, RemindersLogic.repeatSpec(RepeatChoice.None, at(27, 16)))
    }

    @Test fun theWeekdayIsTheOwnersZoneDayNotTheServersOrUtcs() {
        // 23:30 UTC on Friday is already Saturday 02:30 in Kampala.
        val lateUtc = ZonedDateTime.of(2026, 11, 27, 23, 30, 0, 0, ZoneId.of("UTC")).withZoneSameInstant(kampala)
        assertEquals("weekly:6", RemindersLogic.repeatSpec(RepeatChoice.Weekly, lateUtc))
    }

    @Test fun whatHqTakesAsAtIsTheLocalTimeWithoutAnOffset() {
        assertEquals("2026-11-27T16:00", RemindersLogic.localString(at(27, 16)))
        assertEquals("2026-11-27T09:05", RemindersLogic.localString(at(27, 9, 5)))
    }

    @Test fun theListIsGroupedOverdueTodayTomorrowLaterSoonestFirst() {
        val items = listOf(reminder("later", at(30, 9)), reminder("tomorrow", at(28, 9)), reminder("today-b", at(27, 18)), reminder("overdue", at(27, 8)), reminder("today-a", at(27, 15)))
        val groups = RemindersLogic.grouped(items, now)
        assertEquals(listOf(Group.Overdue, Group.Today, Group.Tomorrow, Group.Later), groups.map { it.first })
        assertEquals(listOf("today-a", "today-b"), groups[1].second.map { it.id })
        assertEquals(listOf(Group.Later), RemindersLogic.grouped(listOf(items[0]), now).map { it.first })
        assertTrue(RemindersLogic.grouped(emptyList(), now).isEmpty())
    }

    @Test fun lateTonightInUtcIsTomorrowInKampala() {
        val due = ZonedDateTime.of(2026, 11, 27, 22, 0, 0, 0, ZoneId.of("UTC")).toEpochSecond()
        assertEquals(Group.Tomorrow, RemindersLogic.group(due, now))
    }

    @Test fun theZoneHqSavedWinsAndABadOrMissingOneFallsBackToThePhone() {
        assertEquals(kampala, RemindersLogic.zoneOf("Africa/Kampala", london))
        assertEquals(london, RemindersLogic.zoneOf(null, london))
        assertEquals(london, RemindersLogic.zoneOf("", london))
        assertEquals(london, RemindersLogic.zoneOf("Mars/Olympus", london))
    }
}
