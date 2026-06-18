package io.liberalize.dogtag.data

/** A credential group as shown on Home (Health / Service / Travel). */
enum class CredentialGroup(val title: String) {
    Health("Health Records"),
    Service("Service Dog"),
    Travel("Travel Docs"),
}

/** A single credential / record held for a pet. */
data class Credential(
    val id: String,
    val group: CredentialGroup,
    val recordType: String,   // e.g. "Vaccine", "DOT Service Dog Form"
    val title: String,
    val subtitle: String,
    val issuer: String,
    val issuedOn: String,
)

/** The pet card (reference: "Blaze", Goldendoodle). */
data class Pet(
    val name: String,
    val breed: String,
    val ageLabel: String,
    val dogTagId: String,     // on-chain DogTagSBT tokenId (decimal)
)

/** In-memory seed data mirroring the reference app so the screens are populated out of the box. */
object DemoData {
    val pet = Pet(
        name = "Blaze",
        breed = "Goldendoodle",
        ageLabel = "2 yrs 7 mo",
        dogTagId = "42",
    )

    val credentials = listOf(
        Credential("h1", CredentialGroup.Health, "Vaccine", "Rabies (3-yr)", "Lot #RB-2291 · valid to 2027", "Liberalize Vet Clinic", "2024-08-14"),
        Credential("h2", CredentialGroup.Health, "Vaccine", "DHPP Booster", "Annual core vaccine", "Liberalize Vet Clinic", "2024-08-14"),
        Credential("h3", CredentialGroup.Health, "Checkup / Wellness", "Annual Wellness Exam", "Healthy · 28.4 kg", "Liberalize Vet Clinic", "2025-03-02"),
        Credential("h4", CredentialGroup.Health, "Lab Work", "Heartworm Antigen", "Negative", "IDEXX Reference Labs", "2025-03-02"),
        Credential("s1", CredentialGroup.Service, "DOT Service Dog Form", "Service Animal Attestation", "DOT Air Transportation Form", "Owner-attested", "2025-01-10"),
        Credential("t1", CredentialGroup.Travel, "CDC Dog Import Form", "U.S. Entry Receipt", "Valid 6 months", "CDC", "2025-05-20"),
        Credential("t2", CredentialGroup.Travel, "Microchip", "ISO 11784/11785", "985112… (15 digit)", "Liberalize Vet Clinic", "2022-11-03"),
        Credential("t3", CredentialGroup.Travel, "Rabies Certificate", "International Travel", "EU/UK accepted", "Liberalize Vet Clinic", "2024-08-14"),
        Credential("t4", CredentialGroup.Travel, "Health Certificate", "USDA-endorsed", "10-day validity", "USDA APHIS", "2025-05-18"),
    )

    fun countFor(group: CredentialGroup) = credentials.count { it.group == group }
}
