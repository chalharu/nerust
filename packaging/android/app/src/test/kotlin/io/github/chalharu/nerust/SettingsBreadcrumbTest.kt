package io.github.chalharu.nerust

import org.junit.Assert.assertEquals
import org.junit.Test

/** The projection is a tree: the Devices gear is the single path into
 * pad screens, pads never link back, so the parent map has one parent
 * per child and the breadcrumb walk ends at the top level. */
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

    private fun treePadFixture(): List<SettingsSection> =
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
                rows = emptyList(),
                visible = false,
            ),
        )

    @Test(timeout = 5000)
    fun parentMapFollowsTheGearDownToThePad() {
        val parents = parentMap(treePadFixture())
        assertEquals("devices.p1", parents["pad.1"]?.first)
        assertEquals("player.1", parents["devices.p1"]?.first)
        assertEquals("controllers", parents["player.1"]?.first)
    }

    @Test(timeout = 5000)
    fun breadcrumbClimbsToTheVisibleCategory() {
        val parents = parentMap(treePadFixture())
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
    fun firstLinkWinsWhenTwoSectionsClaimOneChild() {
        val sections = treePadFixture() +
            SettingsSection(
                id = "stray",
                label = "Stray",
                rows = listOf(groupRow("stray.pad1", "Pad A", "pad.1")),
                visible = false,
            )
        assertEquals("devices.p1", parentMap(sections)["pad.1"]?.first)
    }
}
