package io.github.chalharu.nerust

import java.io.FilterInputStream
import java.io.IOException
import java.io.InputStream

/** Unknown provider sizes are checked against the committed generation at EOF. */
internal class SafVerifiedInputStream(stream: InputStream, private val expected: Long) : FilterInputStream(stream) {
    private var count = 0L
    private fun verify(read: Int): Int {
        if (read < 0) {
            if (count != expected) throw IOException("Incomplete SAF generation")
        } else {
            count += read
            if (count > expected) throw IOException("Oversized SAF generation")
        }
        return read
    }
    override fun read(): Int {
        val value = `in`.read()
        verify(if (value < 0) -1 else 1)
        return value
    }
    override fun read(bytes: ByteArray, offset: Int, length: Int): Int =
        verify(`in`.read(bytes, offset, length))
}