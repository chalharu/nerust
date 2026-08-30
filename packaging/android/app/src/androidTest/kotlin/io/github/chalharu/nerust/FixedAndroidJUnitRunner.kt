package io.github.chalharu.nerust

import android.os.Bundle
import android.util.Log
import androidx.test.runner.AndroidJUnitRunner

class FixedAndroidJUnitRunner : AndroidJUnitRunner() {
    override fun onCreate(arguments: Bundle?) {
        patchUtf8Charset()
        super.onCreate(arguments)
    }

    override fun onStart() {
        patchUtf8Charset()
        super.onStart()
    }

    private fun patchUtf8Charset() {
        try {
            val utf8 = java.nio.charset.Charset.forName("UTF-8")
            val charsetClass = java.nio.charset.Charset::class.java
            var patched = false
            for (field in charsetClass.declaredFields) {
                if (!java.util.Map::class.java.isAssignableFrom(field.type)) continue
                field.isAccessible = true
                @Suppress("UNCHECKED_CAST")
                val map = field.get(null) as? MutableMap<String, java.nio.charset.Charset> ?: continue
                val looksLikeCache = map.containsKey("UTF-8") || map.containsKey("utf-8") || map.containsKey("UTF8")
                if (!looksLikeCache && map.isNotEmpty()) continue
                if (map.containsKey("UTF_8")) continue
                map["UTF_8"] = utf8
                map["utf_8"] = utf8
                Log.i("FixedRunner", "Patched Charset cache field ${field.name} for UTF_8")
                patched = true
                break
            }
            if (!patched) {
                try {
                    val cacheField = charsetClass.getDeclaredField("cache")
                    cacheField.isAccessible = true
                    @Suppress("UNCHECKED_CAST")
                    val map = cacheField.get(null) as MutableMap<String, java.nio.charset.Charset>
                    if (!map.containsKey("UTF_8")) {
                        map["UTF_8"] = utf8
                        map["utf_8"] = utf8
                        Log.i("FixedRunner", "Patched Charset.cache for UTF_8")
                    }
                } catch (_: Throwable) {
                }
            }
        } catch (error: Throwable) {
            Log.w("FixedRunner", "Failed to patch UTF_8 charset alias", error)
        }
    }
}
