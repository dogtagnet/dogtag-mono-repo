package io.liberalize.dogtag.net

import org.json.JSONObject
import java.net.URLEncoder

/**
 * Phone-side DNS verification of the groomer (architecture §13.3 H, mirrors `stacks/admin/api/src/dns.rs`).
 *
 * The export QR carries the groomer's wallet/relayer address; before the phone discloses a proof it
 * requires the groomer's DOMAIN to publish a TXT record binding that domain to the address:
 *   `dogtag-verify=<groomerAddr lowercased>`
 * resolved via DNS-over-HTTPS (Cloudflare). This is enforced ONLY for real public domains; the LOCAL
 * demo (IP literals / localhost / *.local / private-LAN hosts) SKIPS the check entirely.
 */
object DnsVerify {
    /** The canonical TXT a groomer must publish to prove control of its domain. */
    fun expectedTxt(groomerAddr: String): String = "dogtag-verify=${groomerAddr.lowercase()}"

    /**
     * True when `host` is a LOCAL host (IP literal / localhost / *.local / private-LAN IP), for which the
     * DNS check is skipped (the local demo). `host` may be a full origin like `https://10.0.0.5:8787`.
     */
    fun isLocalHost(host: String): Boolean {
        val h = hostOnly(host).lowercase()
        if (h.isBlank()) return true
        if (h == "localhost" || h.endsWith(".local") || h.endsWith(".localhost")) return true
        // dev tunnels used to expose the LOCAL demo to a phone (can't host a dogtag-verify TXT) → skip,
        // same as any other local host. DNS-verify stays enforced for real groomer domains in prod.
        if (h.endsWith(".trycloudflare.com") || h.endsWith(".ngrok-free.app") || h.endsWith(".ngrok.io") || h.endsWith(".loca.lt")) return true
        // IPv6 loopback / link-local (bracketed origins already stripped to the literal).
        if (h == "::1" || h.startsWith("fe80:") || h.startsWith("fc") || h.startsWith("fd")) return true
        // IPv4 dotted-quad → inspect the private/loopback ranges.
        val octets = h.split(".")
        if (octets.size == 4 && octets.all { it.toIntOrNull() in 0..255 }) {
            val a = octets[0].toInt(); val b = octets[1].toInt()
            if (a == 127) return true                       // loopback 127/8
            if (a == 10) return true                        // 10/8
            if (a == 192 && b == 168) return true           // 192.168/16
            if (a == 172 && b in 16..31) return true        // 172.16/12
            if (a == 169 && b == 254) return true           // link-local 169.254/16
            if (a == 0) return true
            return false                                    // any other IPv4 literal = public
        }
        return false
    }

    /** Strip scheme/port/path from an origin or host string → bare host. */
    fun hostOnly(host: String): String {
        var h = host.trim()
        val scheme = h.indexOf("://")
        if (scheme >= 0) h = h.substring(scheme + 3)
        h = h.substringBefore('/')
        // strip IPv6 brackets
        if (h.startsWith("[")) {
            val close = h.indexOf(']')
            if (close > 0) return h.substring(1, close)
        }
        // strip :port (IPv4 / hostname)
        val colon = h.lastIndexOf(':')
        if (colon > 0 && h.indexOf(':') == colon) h = h.substring(0, colon)
        return h
    }

    /**
     * Resolve the groomer's domain via DoH and require a TXT answer CONTAINING
     * `dogtag-verify=<groomerAddr lowercased>`. Returns true iff the binding is published.
     * For LOCAL hosts this returns true (skip — the caller should gate via [isLocalHost]).
     */
    suspend fun verifyGroomer(host: String, groomerAddr: String): Boolean {
        if (isLocalHost(host)) return true
        val domain = hostOnly(host)
        if (domain.isBlank() || groomerAddr.isBlank()) return false
        val expected = expectedTxt(groomerAddr)
        return try {
            val name = URLEncoder.encode(domain, "UTF-8")
            val url = "https://cloudflare-dns.com/dns-query?name=$name&type=TXT"
            val resp = Http.getJsonAccept(url, accept = "application/dns-json")
            if (!resp.ok) return false
            val answers = JSONObject(resp.body).optJSONArray("Answer") ?: return false
            (0 until answers.length()).any { i ->
                val data = answers.getJSONObject(i).optString("data", "").trim('"')
                data.contains(expected)
            }
        } catch (e: Exception) {
            false
        }
    }
}
