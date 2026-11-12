package io.github.chalharu.nerust

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SlotMenuTest {
    @Test
    fun parsesOccupiedSlotsWithActive() {
        val raw =
            JSONObject()
                .put("slots", org.json.JSONArray().put(slotJson(1, 1_700_000_000_000)).put(slotJson(2, null)))
                .put("active", 2)
                .toString()
        val rows = parseSlotList(raw)
        assertEquals(2, rows.size)
        assertEquals(SlotEntry(1, 1_700_000_000_000, occupied = true, active = false), rows[0])
        assertEquals(SlotEntry(2, null, occupied = true, active = true), rows[1])
    }

    @Test
    fun appendsUnsavedActiveSlot() {
        val raw = JSONObject().put("slots", org.json.JSONArray().put(slotJson(1, 1_700_000_000_000))).put("active", 5).toString()
        val rows = parseSlotList(raw)
        assertEquals(2, rows.size)
        assertEquals(SlotEntry(5, null, occupied = false, active = true), rows[1])
    }

    @Test
    fun malformedPayloadShowsNoRows() {
        assertTrue(parseSlotList("not json").isEmpty())
        assertTrue(parseSlotList("{}").isEmpty())
        assertTrue(parseSlotList("{\"slots\":[{\"saved_at_ms\":1}]}").isEmpty())
    }

    @Test
    fun timeFormatsAsLocalDateTime() {
        assertTrue(formatSlotTime(1_700_000_000_000).matches(Regex("\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}")))
    }

    private fun slotJson(id: Long, savedAtMs: Long?): JSONObject =
        JSONObject().put("id", id).put("saved_at_ms", savedAtMs ?: JSONObject.NULL)
}
