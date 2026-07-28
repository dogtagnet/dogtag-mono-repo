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
        // Whether the ABSENCE of an owner-secret record for a tag is established at all. Only a
        // store that answered can license the definite negative; under [OwnedTagSource.Unreadable]
        // and [OwnedTagSource.Pending] every row's `own` is null for a reason that says nothing
        // about that row.
        val storeAnswered = owned is OwnedTagSource.Records

        // Imported pets are the source that NEEDS this filter: `RecordImporter` stores a 32-hex
        // share token in `dogTagId` whenever the wrapped doc carries no handle, and that is not a
        // tag. Owner-secret records cannot carry one - `ProfileTreeBuilder.dogTagIdField` refuses
        // anything but a decimal handle - so for them the same predicate is a no-op today. It is
        // applied to both anyway: one predicate cannot drift, whereas two that merely happen to
        // agree on every reachable input is how a mirrored pair starts listing different rows.
        val importedByTag = imported
            .filter { isAsciiDecimal(it.dogTagIdDec) }
            .associateBy { it.dogTagIdDec }

        val ownedByTag = ownedTags
            .filter { isAsciiDecimal(it.dogTagIdDec) }
            .associateBy { it.dogTagIdDec }

        val rows = (ownedByTag.keys + importedByTag.keys).map { id ->
            val own = ownedByTag[id]
            val imp = importedByTag[id]
            DogTagRow(
                dogTagIdDec = id,
                name = realName(imp?.name),
                rootHex = own?.rootHex,
                ownerSecret = when {
                    own != null -> OwnerSecretEvidence.Held
                    storeAnswered -> OwnerSecretEvidence.NotHeld
                    else -> OwnerSecretEvidence.Unknown
                },
                credentialImported = imp != null,
            )
        }.sortedWith { a, b -> descendingHandle(a.dogTagIdDec, b.dogTagIdDec) }

        return DogTagCardState(
            rows = rows,
            ownerStoreUnavailable = (owned as? OwnedTagSource.Unreadable)
                ?.let { shortReason(reasonText(it.cause)) },
            ownerStorePending = owned is OwnedTagSource.Pending,
        )
    }

    /**
     * What the card says when the owner-secret store did not answer - CONSTRUCTED here, never an
     * underlying failure's own message.
     *
     * The store this reports on holds the owner-secret and the attribute salts, and by the time a
     * DECODE fails the decryption has already SUCCEEDED, so the throwable is handed that plaintext:
     * `org.json` quotes what it choked on (`JSONTokener.syntaxError` appends the tokenizer input
     * outright). Rendering it on screen would contradict the one property the whole owner-hidden
     * model rests on. Rather than reason about how much of it any given failure happens to expose,
     * the payload of [OwnedTagSource.Unreadable] is a closed set of causes rather than a string:
     * there is no caller text for this to echo, by construction rather than by convention. iOS
     * mirrors the discipline for the same reason, though its `DecodingError` carries the coding
     * path - field names and indices - rather than values.
     *
     * Both sentences still state the consequence - a tag created by issuance is missing from the list
     * - because saying nothing would recreate the false absence this card exists to close.
     */
    internal fun reasonText(cause: OwnerStoreFailure): String = when (cause) {
        // No "unlock and retry" here: this cause also covers an invalidated Keystore key, where
        // retrying can never work, and promising a remedy that cannot deliver is the same
        // over-claim in a smaller place.
        OwnerStoreFailure.CouldNotRead ->
            "Could not read this device's owner-secret store, so any tag created by issuance is " +
                "missing from this list. The device may be locked, or the store damaged."
        OwnerStoreFailure.CouldNotDecode ->
            "This device's owner-secret store was read but its contents did not decode, so any " +
                "tag created by issuance is missing from this list."
    }

    /** How much of a store-read failure the card prints. See [shortReason]. */
    internal const val MAX_REASON_CHARS = 160

    /**
     * A store-read message collapsed to one printable line.
     *
     * Applied here rather than at the renderer so neither platform can forget it, and retained as
     * the residual cap now that [reasonText] is the only thing feeding it: whatever a future cause's
     * sentence grows into, the settings card gets one line rather than a paragraph that buries the
     * part that matters ("any tag created by issuance is missing from this list"). Nothing
     * actionable is lost to the cap - the remedy does not vary with the tail, the tags are still on
     * chain, and recovery needs the seed plus the credential either way.
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
        if (!isAsciiDecimal(s)) return null
        return s.trimStart('0').ifEmpty { "0" }
    }

    /**
     * A non-empty run of ASCII `0`-`9`, which is what a `dogTagId` handle actually is.
     *
     * Deliberately NOT `Char::isDigit` / Swift's `\.isNumber`: both admit Unicode digits beyond
     * ASCII, and they admit DIFFERENT ones - `isDigit` is category Nd (so `٣` U+0663 passes) while
     * `\.isNumber` adds Nl and No (so `½` and `Ⅸ` pass too). A shared module whose two halves accept
     * different handles would list different rows on the two platforms for one identical store,
     * which is the exact drift this module exists to prevent. Nothing reaches either source with a
     * non-ASCII digit today, so this is a tightening rather than a behaviour change.
     */
    private fun isAsciiDecimal(s: String): Boolean = s.isNotEmpty() && s.all { it in '0'..'9' }
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
     *
     * The payload is a CAUSE, never the failure's own message - see [DogTagCard.reasonText].
     */
    data class Unreadable(val cause: OwnerStoreFailure) : OwnedTagSource

    /** Not asked yet - the read is Keystore crypto plus file I/O and runs off the main thread. */
    data object Pending : OwnedTagSource
}

/**
 * Why the owner-secret store could not be read - a closed set the code itself names, so that what
 * the card prints is always constructed rather than quoted from a throwable. See
 * [DogTagCard.reasonText] for why quoting one would be a privacy defect rather than a copy nit.
 */
enum class OwnerStoreFailure {
    /** The stored bytes never came back: a locked device, an invalidated key, or file I/O. */
    CouldNotRead,

    /** They came back, but did not decode into owner-secret records. */
    CouldNotDecode,
}

/**
 * What is known about this device's ability to prove consent for one listed tag.
 *
 * Three cases for the same reason [OwnedTagSource] has three: a row whose owner-secret record is
 * missing because the store said so is a different claim from one whose record is missing because
 * the store never answered, and only the first may be stated as a fact. Collapsing this to a
 * boolean re-introduces the card's own defect one level down - a definite negative asserted over a
 * source that was never read.
 */
enum class OwnerSecretEvidence {
    /** The store answered and holds this tag's owner-secret, so consent can still be proved here. */
    Held,

    /** The store answered and holds no record for this tag. A definite negative, safe to state. */
    NotHeld,

    /** The store did not answer. Unestablished - the card must claim nothing either way. */
    Unknown,
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
    /**
     * `R`, known only for a tag whose owner-secret this device holds - and therefore `null` both
     * when no such record exists AND when the store did not answer. A `null` root renders no row at
     * all rather than a negative one, so it asserts nothing on its own; [ownerSecret] is what says
     * whether the store was in a position to answer.
     */
    val rootHex: String?,
    /** Whether this device holds the owner-secret for this tag - or could not establish it. */
    val ownerSecret: OwnerSecretEvidence,
    /** A credential naming this tag has been imported here. */
    val credentialImported: Boolean,
)

/** Everything the card renders, including whether it was able to check at all. */
data class DogTagCardState(
    val rows: List<DogTagRow>,
    /** The constructed sentence for a store that could not be read, or `null` when it answered. */
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
