package io.liberalize.dogtag.data

/** Fixed centralized infrastructure; chain peers are a persisted user setting. */
object AppConfig {
    /**
     * Provider search. This centralized/indexer endpoint is deliberately not user-configurable -
     * the owner can repoint the chain RPC but not this, so it is the one service that sees every
     * search. Nearby sends the current position in a POST body and never in a URL or query.
     */
    const val CENTRAL_API = "https://api.dogtag.io"
}
