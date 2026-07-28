package io.liberalize.dogtag.profile

/**
 * What the Profile screen's **Dog-tags** card is allowed to state, derived from the two independent
 * places this device can learn that a tag exists.
 *
 * # Why this exists at all
 *
 * The card used to render from the imported-*pets* list alone, and **custodial issuance never writes
 * a pet**. So a phone that had scanned a real vet QR, folded the profile tree on-device from its own
 * wallet seed, posted `R`, and watched the vet anchor and mint it - with `profileRoot(dogTagId)` on
 * chain equal to the root the phone built - still said *"No dog tag yet"*. It survived an app
 * restart, because the two stores simply never met: [ProfileTreeStore.buildAndPersist] upserts an
 * [OwnerSecretRecord] and nothing on the custodial-bind path adds a pet, while `pets` is populated
 * by the separate record-import QR.
 *
 * That is not a cosmetic gap. The owner holds a minted tag on chain and its owner-secret on this
 * device, and the product's own surface denies it - a false statement about the owner's own
 * property, rendered as absence. **A surface states an accurate observation, or says it could not
 * check**, and an empty card is an accurate observation only when every source actually answered.
 *
 * # Why the card reads the store's THROWING accessor
 *
 * [ProfileTreeStore.all] swallows [ProfileTreeStore.UnreadableStoreException] and answers
 * `emptyList()` - its own doc says "use [ProfileTreeStore.load] where failure must surface". Here it
 * must: a store that exists but cannot be decrypted (corrupt, a newer schema, an invalidated
 * Keystore key) is precisely the case where "No dog tag yet" would be the same false absence in a
 * second flavour. [OwnedTagSource.Unreadable] keeps that distinct from a store that answered
 * "none", and [OwnedTagSource.Pending] keeps it distinct from a read that has not happened yet -
 * the decryption is Keystore AES-GCM plus file I/O, so it runs off the main thread and the card is
 * composed at least once before it lands.
 *
 * # Why it is pure
 *
 * No `Context`, no Keystore, no Compose, no `org.json`: the merge and the three-way "answered /
 * could-not-read / not-yet-asked" distinction are exactly the logic worth pinning, and iOS's
 * `DogTagCard.swift` is a case-for-case mirror so the two platforms cannot drift on what the card
 * claims. [ProfileTreeStore] and `ProfileScreen` supply the I/O around it.
 */
object DogTagCard {

    /**
     * Names the importer writes when it has none - the exact set iOS `LocalStore.isRealName`
     * rejects. `Pet.fromJson`/`fromCentral` default [ImportedTag.name] to the literal `"Unnamed"`
     * (`Models.kt`), and the on-import fallback writes `"DogTag #<id>"`, so a card that trusted the
     * raw string would render a placeholder as though it were the pet's actual name on Android
     * while iOS correctly showed nothing.
     */
    private val PLACEHOLDER_NAME_PREFIXES = listOf("DogTag #", "DogTag#")

    /**
     * The stored name if it is genuinely a name, else `null` - never a substitute that reads like
     * data. Mirrors iOS `LocalStore.isRealName`; keep the two in step.
     */
    fun realName(raw: String?): String? {
        val trimmed = raw?.trim().orEmpty()
        if (trimmed.isEmpty() || trimmed == "Unnamed") return null
        if (PLACEHOLDER_NAME_PREFIXES.any { trimmed.startsWith(it) }) return null
        return trimmed
    }

    /**
     * Fold the two sources into what the card may say.
     *
     * The union is keyed by the decimal handle, so a tag known to BOTH sources is listed once, with
     * each source contributing what only it knows: the owner-secret store contributes `R` and the
     * fact that this device can still prove consent for the tag, the imported credential contributes
     * the pet's name.
     */
    fun state(owned: OwnedTagSource, imported: List<ImportedTag>): DogTagCardState {
        val ownedTags = (owned as? OwnedTagSource.Records)?.tags.orEmpty()

        // Imported pets are the weaker source and need filtering: `RecordImporter` stores a 32-hex
        // share token in `dogTagId` whenever the wrapped doc carries no handle, and that is not a
        // tag. Owner-secret records need no such filter - `ProfileTreeBuilder.dogTagIdField` refuses
        // anything but a decimal handle, so a record in the store is a real tag by construction.
        val importedByTag = imported
            .filter { it.dogTagIdDec.isNotBlank() && it.dogTagIdDec.all(Char::isDigit) }
            .associateBy { it.dogTagIdDec }

        val ownedByTag = ownedTags
            .filter { it.dogTagIdDec.isNotBlank() }
            .associateBy { it.dogTagIdDec }

        val rows = (ownedByTag.keys + importedByTag.keys).map { id ->
            val own = ownedByTag[id]
            val imp = importedByTag[id]
            DogTagRow(
                dogTagIdDec = id,
                name = realName(imp?.name),
                rootHex = own?.rootHex,
                ownerSecretHeld = own != null,
                credentialImported = imp != null,
            )
        }.sortedWith { a, b -> descendingHandle(a.dogTagIdDec, b.dogTagIdDec) }

        return DogTagCardState(
            rows = rows,
            ownerStoreUnavailable = (owned as? OwnedTagSource.Unreadable)?.let { shortReason(it.reason) },
            ownerStorePending = owned is OwnedTagSource.Pending,
        )
    }

    /** How much of a store-read failure the card prints. See [shortReason]. */
    internal const val MAX_REASON_CHARS = 160

    /**
     * A store-read failure collapsed to one printable line.
     *
     * Applied here rather than at the renderer so neither platform can forget it. The underlying
     * throwables are verbose - iOS's `DecodingError` alone renders ~400 characters of nested
     * `Context(codingPath:…)` - and pasting that into a settings card buries the sentence that
     * matters ("any tag created by issuance is missing from this list") under a stack of Foundation
     * internals. The head is the part worth keeping: it names the file and the class of failure.
     * Nothing actionable is lost, because the remedy does not vary with the parse detail - the tags
     * are still on chain, and recovery needs the seed plus the credential either way.
     */
    internal fun shortReason(raw: String): String {
        val collapsed = raw.replace(Regex("\\s+"), " ").trim()
        if (collapsed.length <= MAX_REASON_CHARS) return collapsed
        return collapsed.take(MAX_REASON_CHARS).trimEnd() + "…"
    }

    /**
     * Highest handle first: ids are allocated in ascending order, so the tag the owner just had
     * issued - the one they opened this card to find - is the one at the top.
     *
     * Ordered on the digit string rather than through a fixed-width integer. `dogTagIdDec` is the
     * operator-facing handle and small today, but nothing in the type bounds it, and an overflowing
     * parse would silently reorder the list rather than fail. Longer normalized digit string means
     * larger; same length compares lexicographically, which for equal-length digits IS the numeric
     * order. Deliberately the same five lines as iOS `DogTagCard.descendingHandle` - a `BigInteger`
     * here against a hand-rolled compare there would be two algorithms to keep agreeing.
     */
    private fun descendingHandle(a: String, b: String): Int {
        val na = normalizedHandle(a)
        val nb = normalizedHandle(b)
        return when {
            na != null && nb != null ->
                if (na.length != nb.length) nb.length - na.length else nb.compareTo(na)
            // A non-numeric handle cannot reach here from either source; it sorts last, never throws.
            na != null -> -1
            nb != null -> 1
            else -> a.compareTo(b)
        }
    }

    /** Digits with leading zeros stripped, or `null` when the handle is not a decimal number. */
    private fun normalizedHandle(s: String): String? {
        if (s.isEmpty() || !s.all(Char::isDigit)) return null
        return s.trimStart('0').ifEmpty { "0" }
    }
}

/** A tag this device created by issuance - the owner-secret store's view of it. */
data class OwnedTag(
    /** The human-facing decimal handle. */
    val dogTagIdDec: String,
    /** `R`, the write-once profile root the issuer anchored for this tag. */
    val rootHex: String,
)

/** A tag this device knows of because a credential naming it was imported. */
data class ImportedTag(
    val dogTagIdDec: String,
    /** The raw stored name; [DogTagCard.realName] decides whether it is genuinely one. */
    val name: String,
)

/**
 * What the device-local owner-secret store answered when the card asked it.
 *
 * Three cases, not two, because "there are no tags", "I could not read the store" and "I have not
 * looked yet" are three different claims and only the first of them licenses an empty card.
 */
sealed interface OwnedTagSource {
    /** It answered. An empty list genuinely means this device created no tag. */
    data class Records(val tags: List<OwnedTag>) : OwnedTagSource

    /**
     * It did not answer: the store exists but could not be decrypted or parsed. Any tag created by
     * issuance is therefore unlisted, and the card must say so instead of reporting absence.
     */
    data class Unreadable(val reason: String) : OwnedTagSource

    /** Not asked yet - the read is Keystore crypto plus file I/O and runs off the main thread. */
    data object Pending : OwnedTagSource
}

/**
 * One tag the card lists, and exactly what is known about it.
 *
 * A tag can be known through issuance without any credential having been imported for it, so
 * [name] is genuinely absent rather than filled with a stand-in.
 */
data class DogTagRow(
    val dogTagIdDec: String,
    /** The pet's name, or `null` when this device does not know one. Never a placeholder. */
    val name: String?,
    /** `R`, known only for a tag whose owner-secret this device holds. */
    val rootHex: String?,
    /** This device holds the owner-secret, so it can still prove consent for this tag. */
    val ownerSecretHeld: Boolean,
    /** A credential naming this tag has been imported here. */
    val credentialImported: Boolean,
)

/** Everything the card renders, including whether it was able to check at all. */
data class DogTagCardState(
    val rows: List<DogTagRow>,
    /** Why the owner-secret store could not be read, or `null` when it answered. */
    val ownerStoreUnavailable: String?,
    /** The owner-secret store has not been read yet. */
    val ownerStorePending: Boolean,
) {
    /**
     * The ONE condition under which the card may state that there is no dog tag: every source
     * answered, and none of them knows one. An unread or unreadable store makes absence unproven,
     * so the card reports what it could not check instead of asserting a negative.
     */
    val establishesNoTags: Boolean
        get() = rows.isEmpty() && ownerStoreUnavailable == null && !ownerStorePending
}
