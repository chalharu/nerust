package io.github.chalharu.nerust

import android.content.Context
import android.content.res.Configuration
import java.util.Locale

/** In-app language persistence and locale wrapping. Android resources
 * (drawer, dialogs) resolve through the wrapped context, so they follow
 * the saved settings choice after a recreate; empty means the terminal
 * locale. Rust-rendered strings resolve separately through the vault
 * locale push (see `pushSystemLocale`). */
internal object AppLocale {
    private const val PREFS = "nerust_app"
    private const val KEY = "app_locale"

    /** BCP-47 display tag, "" for the terminal locale. */
    internal fun savedTag(context: Context): String =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY, "") ?: ""

    /** Persists the tag; returns true when the value changed. */
    internal fun setSavedTag(context: Context, tag: String): Boolean {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if ((prefs.getString(KEY, "") ?: "") == tag) return false
        prefs.edit().putString(KEY, tag).apply()
        return true
    }

    /** Wraps the base context with the saved locale (no-op for system). */
    internal fun wrap(base: Context): Context {
        val tag = savedTag(base)
        if (tag.isEmpty()) return base
        val config = Configuration(base.resources.configuration)
        config.setLocale(Locale.forLanguageTag(tag))
        return base.createConfigurationContext(config)
    }

    /** Terminal locale tag for the Rust vault (`null` when unknown). */
    internal fun currentTag(context: Context): String? {
        val locales = context.resources.configuration.locales
        if (locales.isEmpty) return null
        return locales[0].language.takeIf { it.isNotEmpty() }
    }
}
