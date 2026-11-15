package io.github.chalharu.nerust

import org.junit.Assert.assertEquals
import org.junit.Test

/** Regression: the pad screen and its Devices radio link to each other,
 * so a naive parent map cycles pad <-> Devices and the search breadcrumb
 * walk never ends (UI-thread hang on any search hit while an assigned
 * pad is connected). The map must drop cycle-closing edges and the walk
 * itself must carry a visited set. */
class SettingsBreadcrumbTest {
    private fun groupRow(
        key: String,
        label: String,
        target: String,
    ): SettingsRow =
        SettingsRow(
            key = key,
            label = label,
            kind = "group",
            choices = emptyList(),
            selected = 0,
            number = 0L,
            step = 0L,
            unit = "",
            text = "",
            target = target,
        )

    private fun cyclicPadFixture(): List<SettingsSection> =
        listOf(
            SettingsSection(
                id = "controllers",
                label = "Controllers",
                rows =
                    listOf(
                        groupRow("controllers.player.1", "Player 1", "player.1"),
                    ),
                visible = true,
            ),
            SettingsSection(
                id = "player.1",
                label = "Player 1",
                rows =
                    listOf(
                        groupRow("player.1.devices", "Devices", "devices.p1"),
                    ),
                visible = false,
            ),
            SettingsSection(
                id = "devices.p1",
                label = "Devices",
                rows =
                    listOf(
                        groupRow("devices.p1.pad1", "Pad A", "pad.1"),
                    ),
                visible = false,
            ),
            SettingsSection(
                id = "pad.1",
                label = "Pad A",
                rows =
                    listOf(
                        groupRow("pad.1.slot", "Player 1", "devices.p1"),
                    ),
                visible = false,
            ),
        )

    @Test(timeout = 5000)
    fun parentMapDropsTheCycleClosingBackLink() {
        val parents = parentMap(cyclicPadFixture())
        // The gear (Devices -> pad) wins; the slot back-link closes a
        // cycle and must be dropped, so Devices keeps its player parent.
        assertEquals("devices.p1", parents["pad.1"]?.first)
        assertEquals("player.1", parents["devices.p1"]?.first)
        assertEquals("controllers", parents["player.1"]?.first)
    }

    @Test(timeout = 5000)
    fun breadcrumbTerminatesOnTheCyclicPair() {
        val parents = parentMap(cyclicPadFixture())
        assertEquals(
            "Controllers › Player 1 › Devices",
            breadcrumbTrail(parents, "pad.1"),
        )
        assertEquals(
            "Controllers › Player 1",
            breadcrumbTrail(parents, "devices.p1"),
        )
    }

    @Test(timeout = 5000)
    fun breadcrumbTerminatesOnAHandBuiltCycle() {
        // Defense in depth: even a fully cyclic map must not hang.
        val parents =
            mapOf(
                "a" to Triple("b", "B", ""),
                "b" to Triple("a", "A", ""),
            )
        breadcrumbTrail(parents, "a")
        breadcrumbTrail(parents, "b")
    }
}
