package io.liberalize.dogtag.net

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL

/** Minimal JSON HTTP (no third-party client) for the central + vet + ROAX endpoints. */
object Http {
    data class Response(val code: Int, val body: String) {
        val ok: Boolean get() = code in 200..299
    }

    suspend fun postJson(url: String, json: String, bearer: String? = null): Response =
        withContext(Dispatchers.IO) {
            val conn = open(url, "POST", bearer)
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "application/json")
            try {
                conn.outputStream.use { it.write(json.toByteArray()) }
                read(conn)
            } finally {
                conn.disconnect()
            }
        }

    suspend fun getJson(url: String, bearer: String? = null): Response =
        withContext(Dispatchers.IO) {
            val conn = open(url, "GET", bearer)
            try {
                read(conn)
            } finally {
                conn.disconnect()
            }
        }

    /** GET with an explicit Accept header (e.g. DoH `application/dns-json`). */
    suspend fun getJsonAccept(url: String, accept: String): Response =
        withContext(Dispatchers.IO) {
            val conn = open(url, "GET", null)
            conn.setRequestProperty("Accept", accept)
            try {
                read(conn)
            } finally {
                conn.disconnect()
            }
        }

    private fun open(url: String, method: String, bearer: String?): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = method
            connectTimeout = 8000
            readTimeout = 8000
            setRequestProperty("Accept", "application/json")
            if (bearer != null) setRequestProperty("Authorization", "Bearer $bearer")
        }

    private fun read(conn: HttpURLConnection): Response {
        val code = conn.responseCode
        val stream = if (code in 200..299) conn.inputStream else conn.errorStream
        val body = stream?.bufferedReader()?.use { it.readText() } ?: ""
        return Response(code, body)
    }
}
