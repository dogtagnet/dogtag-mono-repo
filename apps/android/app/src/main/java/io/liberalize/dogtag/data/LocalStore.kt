package io.liberalize.dogtag.data

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * A tiny, dependency-free local persistence for pets + imported credentials. Backed by two JSON files
 * in the app's private files dir. There is NO sample/mock data — both lists are legitimately empty
 * until the user imports a record (scans a vet QR) or a central pet sync runs.
 *
 * Pets are keyed by `dogTagId`; credentials are keyed by `id` and reference their pet by `dogTagId`,
 * so the Travel/Documents filters select by pet.
 */
class LocalStore(context: Context) {
    private val petsFile = File(context.filesDir, "pets.json")
    private val credsFile = File(context.filesDir, "credentials.json")

    private val _pets = MutableStateFlow(loadPets())
    val pets: StateFlow<List<Pet>> = _pets

    private val _credentials = MutableStateFlow(loadCreds())
    val credentials: StateFlow<List<Credential>> = _credentials

    // ---- pets ----------------------------------------------------------------------------------

    /** Upsert a pet (keyed by dogTagId). Keeps an existing pet's nicer fields if the new one is blank. */
    fun upsertPet(pet: Pet) {
        val cur = _pets.value.toMutableList()
        val idx = cur.indexOfFirst { it.dogTagId == pet.dogTagId }
        if (idx >= 0) cur[idx] = pet else cur.add(pet)
        _pets.value = cur
        savePets(cur)
    }

    /** Merge a batch of pets from the central backend without clobbering local edits. */
    fun mergeCentralPets(incoming: List<Pet>) {
        val cur = _pets.value.toMutableList()
        for (p in incoming) {
            if (p.dogTagId.isBlank()) continue
            val idx = cur.indexOfFirst { it.dogTagId == p.dogTagId }
            if (idx >= 0) cur[idx] = p else cur.add(p)
        }
        _pets.value = cur
        savePets(cur)
    }

    fun petFor(dogTagId: String): Pet? = _pets.value.firstOrNull { it.dogTagId == dogTagId }

    // ---- credentials ---------------------------------------------------------------------------

    /** Store an imported credential; ensures a placeholder pet exists if we don't know it yet. */
    fun addCredential(cred: Credential) {
        if (petFor(cred.dogTagId) == null && cred.dogTagId.isNotBlank()) {
            upsertPet(Pet(dogTagId = cred.dogTagId, name = "DogTag #${cred.dogTagId}", breed = "", ageLabel = ""))
        }
        val cur = _credentials.value.toMutableList()
        val idx = cur.indexOfFirst { it.id == cred.id }
        if (idx >= 0) cur[idx] = cred else cur.add(cred)
        _credentials.value = cur
        saveCreds(cur)
    }

    fun credentialsFor(dogTagId: String?): List<Credential> =
        if (dogTagId == null) _credentials.value
        else _credentials.value.filter { it.dogTagId == dogTagId }

    fun credentialsFor(dogTagId: String?, group: CredentialGroup): List<Credential> =
        credentialsFor(dogTagId).filter { it.group == group }

    // ---- IO --------------------------------------------------------------------------------------

    private fun loadPets(): List<Pet> = runCatching {
        if (!petsFile.exists()) return emptyList()
        val arr = JSONArray(petsFile.readText())
        (0 until arr.length()).map { Pet.fromJson(arr.getJSONObject(it)) }
    }.getOrDefault(emptyList())

    private fun savePets(list: List<Pet>) {
        val arr = JSONArray().apply { list.forEach { put(it.toJson()) } }
        runCatching { petsFile.writeText(arr.toString()) }
    }

    private fun loadCreds(): List<Credential> = runCatching {
        if (!credsFile.exists()) return emptyList()
        val arr = JSONArray(credsFile.readText())
        (0 until arr.length()).map { Credential.fromJson(arr.getJSONObject(it)) }
    }.getOrDefault(emptyList())

    private fun saveCreds(list: List<Credential>) {
        val arr = JSONArray().apply { list.forEach { put(it.toJson()) } }
        runCatching { credsFile.writeText(arr.toString()) }
    }

    companion object {
        @Volatile private var instance: LocalStore? = null
        fun get(context: Context): LocalStore =
            instance ?: synchronized(this) {
                instance ?: LocalStore(context.applicationContext).also { instance = it }
            }
    }
}
