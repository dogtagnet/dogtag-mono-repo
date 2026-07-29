package io.liberalize.dogtag.data

/** Fixed public infrastructure; credential-service hosts still come from scanned QR origins. */
object AppConfig {
    const val ROAX_RPC = "https://devrpc.roax.net"
    /** Full provider-set directory. Nearby never adds an origin or search query to this URL. */
    const val CENTRAL_API = "https://api.dogtag.io"
}
