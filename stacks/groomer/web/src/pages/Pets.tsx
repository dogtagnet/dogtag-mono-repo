import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  DEMO_CRM_PET,
} from "@dogtag/ui";
import type { ClientPetInput, CrmClient, CrmPet, PetCreateInput } from "@dogtag/ui";
import { PawPrint, Plus, Search, Users,
  Sparkles,
} from "lucide-react";
import { useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";
import { ListPlaceholder, PAGE_SIZE, Pager, useAction, useDebounced, useList } from "../app/crm";

/**
 * The PET directory — pets as a collection in their own right.
 *
 * A pet used to be reachable only through its owner's record, which is the wrong way round for how a
 * shop actually works: the animal is what walks in, often with a tag and a person nobody recognises.
 * So a pet is searchable by its own name, species, breed and DogTag id AND by its owner's name, and
 * every row links to both halves — the pet detail page one way, the owner the other.
 *
 * Search and paging run on the BACKEND (`GET /pets`), like every other list here: one bounded page at
 * a time, re-queried as the operator types.
 */
export function Pets() {
  const { api } = useApp();
  const [params, setParams] = useSearchParams();
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const [creating, setCreating] = useState(false);
  const q = useDebounced(search);
  // Arriving from a client's record scopes the list to that owner's pets. The filter is applied on the
  // BACKEND, and is visible and clearable — an invisible filter reads as an empty collection.
  const clientId = params.get("clientId") ?? "";

  const { page, loading, error, reload } = useList<CrmPet>(
    () => api.listPets({ q, clientId: clientId || undefined, limit: PAGE_SIZE, offset }),
    [q, clientId, offset],
  );
  const ownerName = clientId ? (page?.rows[0]?.clientName ?? "") : "";

  // a new needle restarts paging — page 3 of the previous query is meaningless for a new one
  function onSearch(value: string) {
    setSearch(value);
    setOffset(0);
  }

  const rows = page?.rows ?? [];

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2">
              <PawPrint className="h-5 w-5 text-primary" /> Pets
            </CardTitle>
            <CardDescription>
              Every pet the shop knows, searchable by the pet or by its owner. Open a pet to manage its
              details, its DogTag and its credentials.
            </CardDescription>
          </div>
          <Button onClick={() => setCreating(true)}>
            <Plus className="h-4 w-4" /> New pet
          </Button>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
            <Input
              className="pl-9"
              placeholder="Search pet name, species, breed, DogTag id or owner…"
              value={search}
              onChange={(e) => onSearch(e.target.value)}
              aria-label="Search pets"
            />
          </div>

          {clientId && (
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <Badge variant="neutral">
                Owner: {ownerName || clientId}
              </Badge>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setParams({});
                  setOffset(0);
                }}
              >
                Show all pets
              </Button>
            </div>
          )}

          <ListPlaceholder
            loading={loading}
            error={error}
            empty={rows.length === 0}
            emptyMessage={
              q
                ? `No pets match "${q}".`
                : "No pets yet — add one here, or from a client's record."
            }
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Pet</TableHead>
                  <TableHead>Species / breed</TableHead>
                  <TableHead>Owner</TableHead>
                  <TableHead>DogTag</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((p) => (
                  <TableRow key={p.petId}>
                    <TableCell>
                      <Link
                        to={`/pets/${p.petId}`}
                        className="font-medium text-primary hover:underline"
                      >
                        {p.name}
                      </Link>
                    </TableCell>
                    <TableCell className="text-sm text-muted">
                      {[p.species, p.breed].filter(Boolean).join(" · ") || "—"}
                    </TableCell>
                    <TableCell className="text-sm">
                      {/* The other half of the round trip: from the pet straight to its owner. */}
                      <Link
                        to={`/clients/${p.clientId}`}
                        className="text-primary hover:underline"
                      >
                        {p.clientName || "—"}
                      </Link>
                    </TableCell>
                    <TableCell className="text-sm">
                      {p.dogTagId ? (
                        <Badge variant="outline" className="font-mono">
                          {p.dogTagId}
                        </Badge>
                      ) : (
                        <span className="text-muted">No tag</span>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </ListPlaceholder>

          <Pager
            total={page?.total ?? 0}
            offset={offset}
            limit={page?.limit ?? PAGE_SIZE}
            onOffset={setOffset}
          />
        </CardContent>
      </Card>

      {creating && (
        <PetForm
          onCancel={() => setCreating(false)}
          onCreated={() => {
            setCreating(false);
            reload();
          }}
        />
      )}
    </div>
  );
}

/**
 * The pet's own fields, shared by the create form here and the edit form on the detail page so the
 * two can never drift apart (the same reason `ClientForm` is shared between create and edit).
 *
 * The DogTag id is deliberately NOT here. On create it is left unset and on edit it is untouched:
 * linking a tag is its own action, with its own explanation of what it does and does not do, and
 * folding it into a details form would make attaching a credential look like typing a breed.
 */
export function PetFields({
  value,
  onChange,
  idPrefix,
}: {
  value: ClientPetInput;
  onChange: (patch: Partial<ClientPetInput>) => void;
  idPrefix: string;
}) {
  return (
    <div className="space-y-3">
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="space-y-2">
          <Label htmlFor={`${idPrefix}-name`} required>
            Pet name
          </Label>
          <Input
            id={`${idPrefix}-name`}
            value={value.name}
            onChange={(e) => onChange({ name: e.target.value })}
            placeholder="Rex"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${idPrefix}-species`}>Species</Label>
          <Input
            id={`${idPrefix}-species`}
            value={value.species ?? ""}
            onChange={(e) => onChange({ species: e.target.value })}
            placeholder="dog"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${idPrefix}-breed`}>Breed</Label>
          <Input
            id={`${idPrefix}-breed`}
            value={value.breed ?? ""}
            onChange={(e) => onChange({ breed: e.target.value })}
            placeholder="Standard Poodle"
          />
        </div>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="space-y-2">
          <Label htmlFor={`${idPrefix}-sex`}>Sex</Label>
          <Input
            id={`${idPrefix}-sex`}
            value={value.sex ?? ""}
            onChange={(e) => onChange({ sex: e.target.value })}
            placeholder="male / female"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${idPrefix}-dob`}>Date of birth</Label>
          <Input
            id={`${idPrefix}-dob`}
            type="date"
            value={value.dateOfBirth ?? ""}
            onChange={(e) => onChange({ dateOfBirth: e.target.value })}
          />
        </div>
        {/*
          The microchip DOES belong on this form, unlike the DogTag id above. It is a fact about the
          animal that the owner or a previous vet already established, so typing it is data entry —
          whereas attaching a credential is an act with consequences and keeps its own card.

          Its help text has to say that leaving it blank is fine. Beside a field the portal uses to
          cross-check a credential, silence reads as "we need this", and many animals genuinely have
          no chip — cats routinely are not chipped here.
        */}
        <div className="space-y-2">
          <Label htmlFor={`${idPrefix}-microchip`}>Microchip</Label>
          <Input
            id={`${idPrefix}-microchip`}
            value={value.microchipCode ?? ""}
            onChange={(e) => onChange({ microchipCode: e.target.value })}
            placeholder="985141006580319"
            inputMode="numeric"
          />
          <p className="text-xs text-muted">
            Optional. If recorded, it is checked against the credential when a DogTag is linked.
          </p>
        </div>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`${idPrefix}-notes`}>Notes</Label>
        <textarea
          id={`${idPrefix}-notes`}
          className="min-h-20 w-full rounded-md border border-input bg-surface px-3 py-2 text-sm text-onSurface placeholder:text-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
          value={value.notes ?? ""}
          onChange={(e) => onChange({ notes: e.target.value })}
          placeholder="Temperament, handling notes, allergies…"
        />
      </div>
    </div>
  );
}

const BLANK_PET: ClientPetInput = {
  name: "",
  species: "",
  breed: "",
  sex: "",
  dateOfBirth: "",
  notes: "",
};

/**
 * Create a pet UNDER an owner. The owner is required and is picked by searching the client directory
 * — server-side, like every other search here, so this form works against a real customer book rather
 * than only against however many clients happen to fit in one page.
 */
function PetForm({ onCancel, onCreated }: { onCancel: () => void; onCreated: () => void }) {
  const { api } = useApp();
  const { run, busy } = useAction();
  const [pet, setPet] = useState<ClientPetInput>(BLANK_PET);
  const [ownerSearch, setOwnerSearch] = useState("");
  const [owner, setOwner] = useState<CrmClient | null>(null);
  const ownerQ = useDebounced(ownerSearch);

  // Only query while the operator is actually choosing an owner, and only a short list: this is a
  // picker, not a directory.
  const { page: owners, loading: ownersLoading } = useList<CrmClient>(
    () =>
      owner
        ? Promise.resolve({ rows: [], total: 0, limit: 8, offset: 0 })
        : api.listClients({ q: ownerQ, limit: 8 }),
    [ownerQ, owner?.clientId ?? ""],
  );

  const nameInvalid = pet.name.trim().length === 0;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!owner || nameInvalid) return;
    const body: PetCreateInput = { ...pet, clientId: owner.clientId };
    const ok = await run(() => api.createPet(body), {
      success: `${pet.name.trim()} added to ${owner.name}`,
      failure: "Could not add the pet",
    });
    if (ok) onCreated();
  }

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
        <div>
          <CardTitle className="text-base">New pet</CardTitle>
          <CardDescription>
            A pet belongs to an owner, so pick the owner first. Not in the book yet? Create the client,
            then come back — or add the pet directly on their record.
          </CardDescription>
        </div>
        {env.demoMode && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() =>
              setPet((prev) => ({
                ...prev,
                name: DEMO_CRM_PET.name,
                species: DEMO_CRM_PET.species,
                breed: DEMO_CRM_PET.breed,
                sex: DEMO_CRM_PET.sex,
                dateOfBirth: DEMO_CRM_PET.dateOfBirth,
                notes: DEMO_CRM_PET.notes,
                microchipCode: DEMO_CRM_PET.microchipCode,
              }))
            }
          >
            <Sparkles className="h-4 w-4" /> Fill demo data
          </Button>
        )}
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="pet-owner" required>
              Owner
            </Label>
            {owner ? (
              <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border p-3 text-sm">
                <span className="flex items-center gap-2 text-onSurface">
                  <Users className="h-4 w-4 text-muted" /> {owner.name}
                </span>
                <Button type="button" variant="ghost" size="sm" onClick={() => setOwner(null)}>
                  Change
                </Button>
              </div>
            ) : (
              <div className="space-y-2">
                <div className="relative">
                  <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
                  <Input
                    id="pet-owner"
                    className="pl-9"
                    placeholder="Search the client book by name, email or phone…"
                    value={ownerSearch}
                    onChange={(e) => setOwnerSearch(e.target.value)}
                  />
                </div>
                <div className="divide-y divide-border rounded-md border border-border">
                  {ownersLoading && (owners?.rows.length ?? 0) === 0 ? (
                    <p className="px-3 py-3 text-sm text-muted">Searching…</p>
                  ) : (owners?.rows.length ?? 0) === 0 ? (
                    <p className="px-3 py-3 text-sm text-muted">
                      {ownerSearch
                        ? `No clients match "${ownerSearch}".`
                        : "Start typing to find the owner."}
                    </p>
                  ) : (
                    owners?.rows.map((c) => (
                      <button
                        key={c.clientId}
                        type="button"
                        onClick={() => setOwner(c)}
                        className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-sm hover:bg-surface-muted"
                      >
                        <span className="text-onSurface">{c.name}</span>
                        <span className="text-xs text-muted">{c.email || c.phone || ""}</span>
                      </button>
                    ))
                  )}
                </div>
              </div>
            )}
          </div>

          <PetFields
            value={pet}
            idPrefix="new-pet"
            onChange={(patch) => setPet((p) => ({ ...p, ...patch }))}
          />

          <div className="flex flex-wrap gap-2">
            <Button type="submit" loading={busy} disabled={!owner || nameInvalid}>
              Create pet
            </Button>
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
