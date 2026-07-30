package io.liberalize.dogtag.data

/** Fixed centralized infrastructure; chain peers are a persisted user setting. */
object AppConfig {
    /**
     * Full provider-set directory. This centralized/indexer endpoint is deliberately not
     * user-configurable, and Nearby never adds an origin or search query to it.
     */
    const val CENTRAL_API = "https://api.dogtag.io"
}
