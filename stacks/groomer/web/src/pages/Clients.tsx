import {
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
  DEMO_CRM_CLIENT,
} from "@dogtag/ui";
import type { ClientInput, ClientPetInput, CrmClient } from "@dogtag/ui";
import { Plus, Search, Trash2, Users,
  Sparkles,
} from "lucide-react";
import { useState, type FormEvent } from "react";
import { Link } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";
import {
  FilterBar,
  FilterField,
  ListPlaceholder,
  PAGE_SIZE,
  Pager,
  useAction,
  useDebounced,
  useList,
} from "../app/crm";

/**
 * The customer directory (impl §5.2). Owner particulars + their pets, searchable across owner name,
 * email, phone, pet name and DogTag id.
 *
 * Search and paging run on the BACKEND — the page holds one bounded page at a time and re-queries as
 * the operator types, so the directory stays usable at realistic customer volumes.
 */
export function Clients() {
  const { api } = useApp();
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const [creating, setCreating] = useState(false);
  const q = useDebounced(search);

  const { page, loading, error, reload } = useList<CrmClient>(
    () => api.listClients({ q, limit: PAGE_SIZE, offset }),
    [q, offset],
  );

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
              <Users className="h-5 w-5 text-primary" /> Clients
            </CardTitle>
            <CardDescription>
              Your customers and their pets. Book an appointment from a client, then run the
              vaccination check from that appointment.
            </CardDescription>
          </div>
          <Button onClick={() => setCreating(true)}>
            <Plus className="h-4 w-4" /> New client
          </Button>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* The directory has one filter, so it is bounded rather than stretched: a search box the
              width of a 27" monitor is a worse target than one sized to what you type into it. */}
          <FilterBar>
            <FilterField span="sm:col-span-2 xl:col-span-6" label="Search" htmlFor="client-search">
              <div className="relative">
                <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
                <Input
                  id="client-search"
                  className="pl-9"
                  placeholder="Search name, email, phone, pet or DogTag id…"
                  value={search}
                  onChange={(e) => onSearch(e.target.value)}
                  aria-label="Search clients"
                />
              </div>
            </FilterField>
          </FilterBar>

          <ListPlaceholder
            loading={loading}
            error={error}
            empty={rows.length === 0}
            emptyMessage={
              q ? `No clients match "${q}".` : "No clients yet — add your first one to get started."
            }
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Contact</TableHead>
                  <TableHead>Pets</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((c) => (
                  <TableRow key={c.clientId}>
                    <TableCell>
                      <Link
                        to={`/clients/${c.clientId}`}
                        className="font-medium text-primary hover:underline"
                      >
                        {c.name}
                      </Link>
                    </TableCell>
                    <TableCell className="text-sm text-muted">
                      <div>{c.email || "—"}</div>
                      <div>{c.phone}</div>
                    </TableCell>
                    <TableCell className="text-sm">
                      {c.pets.length === 0 ? (
                        <span className="text-muted">No pets</span>
                      ) : (
                        c.pets.map((p) => p.name).join(", ")
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
        <ClientForm
          title="New client"
          onCancel={() => setCreating(false)}
          onSubmit={async (body) => {
            await api.createClient(body);
            setCreating(false);
            reload();
          }}
        />
      )}
    </div>
  );
}

/** A pet row in the form. `petId` is carried through an edit so its appointment links survive. */
type PetDraft = ClientPetInput & { key: string };

function petDraft(seed?: ClientPetInput): PetDraft {
  return {
    key: Math.random().toString(36).slice(2),
    petId: seed?.petId,
    name: seed?.name ?? "",
    species: seed?.species ?? "dog",
    breed: seed?.breed ?? "",
    sex: seed?.sex ?? "",
    dateOfBirth: seed?.dateOfBirth ?? "",
    notes: seed?.notes ?? "",
    dogTagId: seed?.dogTagId ?? "",
    microchipCode: seed?.microchipCode ?? "",
  };
}

/**
 * Create/edit form for a client and their pets. Shared by the directory (create) and the client
 * detail page (edit) so the two can never drift apart.
 */
export function ClientForm({
  title,
  initial,
  onSubmit,
  onCancel,
}: {
  title: string;
  initial?: CrmClient;
  onSubmit: (body: ClientInput) => Promise<void>;
  onCancel: () => void;
}) {
  const { run, busy } = useAction();
  const [name, setName] = useState(initial?.name ?? "");
  const [email, setEmail] = useState(initial?.email ?? "");
  const [phone, setPhone] = useState(initial?.phone ?? "");
  const [address, setAddress] = useState(initial?.address ?? "");
  const [notes, setNotes] = useState(initial?.notes ?? "");
  const [pets, setPets] = useState<PetDraft[]>(() =>
    initial?.pets.length ? initial.pets.map((p) => petDraft(p)) : [petDraft()],
  );

  const nameInvalid = name.trim() === "";

  function updatePet(key: string, patch: Partial<PetDraft>) {
    setPets((prev) => prev.map((p) => (p.key === key ? { ...p, ...patch } : p)));
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (nameInvalid) return;
    const body: ClientInput = {
      name: name.trim(),
      email: email.trim(),
      phone: phone.trim(),
      address: address.trim(),
      notes,
      // a blank pet row is a not-yet-filled-in row, not an unnamed pet
      pets: pets
        .filter((p) => p.name.trim() !== "")
        .map((p) => ({
          petId: p.petId,
          name: p.name,
          species: p.species,
          breed: p.breed,
          sex: p.sex,
          dateOfBirth: p.dateOfBirth,
          notes: p.notes,
          dogTagId: p.dogTagId?.trim() ? p.dogTagId.trim() : null,
          // MUST be carried, not merely offered. This payload REPLACES the owner's whole pet list,
          // so a field the form omits is a field every client edit silently erases — and erasing a
          // microchip does not fail loudly, it just quietly stops the cross-check firing on every
          // future link. `petDraft` seeds it from the stored pet for the same reason.
          microchipCode: p.microchipCode?.trim() ? p.microchipCode.trim() : null,
        })),
    };
    await run(() => onSubmit(body), {
      success: initial ? "Client updated" : "Client created",
      failure: initial ? "Could not update the client" : "Could not create the client",
    });
  }

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
        <CardTitle>{title}</CardTitle>
        {env.demoMode && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              setName(DEMO_CRM_CLIENT.name);
              setEmail(DEMO_CRM_CLIENT.email);
              setPhone(DEMO_CRM_CLIENT.phone);
              setAddress(DEMO_CRM_CLIENT.address);
              setNotes(DEMO_CRM_CLIENT.notes);
              // Fill the FIRST pet row in place rather than replacing the list: a replacement would
              // drop `petId` on an edit, and this payload REPLACES the owner's whole pet list, so
              // that would silently orphan every existing pet from its links.
              setPets((prev) => {
                const [first, ...rest] = prev;
                const filled = {
                  ...(first ?? petDraft()),
                  name: DEMO_CRM_CLIENT.pet.name,
                  species: DEMO_CRM_CLIENT.pet.species,
                  breed: DEMO_CRM_CLIENT.pet.breed,
                  sex: DEMO_CRM_CLIENT.pet.sex,
                  dateOfBirth: DEMO_CRM_CLIENT.pet.dateOfBirth,
                  notes: DEMO_CRM_CLIENT.pet.notes,
                  microchipCode: DEMO_CRM_CLIENT.pet.microchipCode,
                };
                return [filled, ...rest];
              });
            }}
          >
            <Sparkles className="h-4 w-4" /> Fill demo data
          </Button>
        )}
      </CardHeader>
      <CardContent>
        {/* This form is hosted on a WORKING-SURFACE page (the directory uses the full width), but a
            form is read, not scanned: the fields keep a comfortable measure instead of inheriting
            the list's width. */}
        <form className="max-w-3xl space-y-6" onSubmit={submit}>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="client-name">Name</Label>
              <Input
                id="client-name"
                value={name}
                invalid={nameInvalid && name !== ""}
                onChange={(e) => setName(e.target.value)}
                placeholder="Alice Tan"
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="client-phone">Phone</Label>
              <Input
                id="client-phone"
                value={phone}
                onChange={(e) => setPhone(e.target.value)}
                placeholder="+65 9123 4567"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="client-email">Email</Label>
              <Input
                id="client-email"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="alice@example.com"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="client-address">Address</Label>
              <Input
                id="client-address"
                value={address}
                onChange={(e) => setAddress(e.target.value)}
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="client-notes">Notes</Label>
            <textarea
              id="client-notes"
              className="min-h-20 w-full rounded-md border border-input bg-surface px-3 py-2 text-sm text-onSurface placeholder:text-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              placeholder="Preferences, handling notes, allergies…"
            />
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <Label>Pets</Label>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => setPets((p) => [...p, petDraft()])}
              >
                <Plus className="h-4 w-4" /> Add pet
              </Button>
            </div>
            {/*
              A DogTag is linked on the PET page, not here. That page is where linking is explained -
              that it records this shop's own note of which tag the pet holds, mints nothing and
              writes nothing on chain, and that removing it is a local reversible disassociation and
              NOT a revocation. A bare optional field on this form would be a second way in that says
              none of it. Tags already linked are preserved by an edit here; they are just not
              editable from this form.
            */}
            <p className="text-xs text-muted">
              Pet details only. A DogTag is linked from the pet's own page, where what linking does -
              and what it does not do - is spelled out. Any tag already linked stays as it is.
            </p>
            {pets.map((p) => (
              <div key={p.key} className="space-y-3 rounded-md border border-border p-3">
                <div className="grid gap-3 sm:grid-cols-3">
                  <div className="space-y-2">
                    <Label htmlFor={`pet-name-${p.key}`}>Pet name</Label>
                    <Input
                      id={`pet-name-${p.key}`}
                      value={p.name}
                      onChange={(e) => updatePet(p.key, { name: e.target.value })}
                      placeholder="Rex"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor={`pet-breed-${p.key}`}>Breed</Label>
                    <Input
                      id={`pet-breed-${p.key}`}
                      value={p.breed}
                      onChange={(e) => updatePet(p.key, { breed: e.target.value })}
                      placeholder="Standard Poodle"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor={`pet-dob-${p.key}`}>Date of birth</Label>
                    <Input
                      id={`pet-dob-${p.key}`}
                      type="date"
                      value={p.dateOfBirth}
                      onChange={(e) => updatePet(p.key, { dateOfBirth: e.target.value })}
                    />
                  </div>
                </div>
                <div className="grid gap-3 sm:grid-cols-3">
                  <div className="space-y-2">
                    <Label htmlFor={`pet-sex-${p.key}`}>Sex</Label>
                    <Input
                      id={`pet-sex-${p.key}`}
                      value={p.sex}
                      onChange={(e) => updatePet(p.key, { sex: e.target.value })}
                      placeholder="male / female"
                    />
                  </div>
                  {/*
                    Optional, and the help text has to say so: beside a field used to cross-check a
                    credential, silence reads as "required", and many animals genuinely have no chip.
                  */}
                  <div className="space-y-2">
                    <Label htmlFor={`pet-microchip-${p.key}`}>Microchip</Label>
                    <Input
                      id={`pet-microchip-${p.key}`}
                      value={p.microchipCode ?? ""}
                      onChange={(e) => updatePet(p.key, { microchipCode: e.target.value })}
                      placeholder="985141006580319"
                      inputMode="numeric"
                    />
                    <p className="text-xs text-muted">
                      Optional. Checked against the credential when a DogTag is linked.
                    </p>
                  </div>
                </div>
                {pets.length > 1 && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => setPets((prev) => prev.filter((x) => x.key !== p.key))}
                  >
                    <Trash2 className="h-4 w-4" /> Remove pet
                  </Button>
                )}
              </div>
            ))}
          </div>

          <div className="flex flex-wrap gap-2">
            <Button type="submit" loading={busy} disabled={nameInvalid}>
              {initial ? "Save changes" : "Create client"}
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
