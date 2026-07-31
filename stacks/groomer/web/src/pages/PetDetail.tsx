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
  Spinner,
  useToast,
  microchipCheckFromError,
  microchipExplanation,
  microchipHeadline,
  microchipTone,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@dogtag/ui";
import type {
  ClientPetInput,
  CrmAppointment,
  CrmPet,
  CrmVerification,
  MicrochipCheck,
} from "@dogtag/ui";
import {
  ArrowLeft,
  CalendarPlus,
  Link2,
  Link2Off,
  PawPrint,
  Pencil,
  ShieldCheck,
  Trash2,
  User,
} from "lucide-react";
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useApp } from "../app/AppContext";
import {
  AppointmentStatusBadge,
  DisclosureBadge,
  ListPlaceholder,
  VerificationStatusBadge,
  useAction,
} from "../app/crm";
import { formatDateTime } from "../lib/time";
import { PetFields } from "./Pets";
import { PetTagCredentials } from "./PetTagCredentials";
import { TagDiscoveryPanel } from "./TagDiscoveryPanel";

/**
 * ONE PET, managed.
 *
 * A pet used to be a read-only block on its owner's record. This is the page that makes it an entity:
 * its details are editable here, its DogTag is linked and unlinked here, its credentials are checked
 * against the chain here, and the chain can be searched from here for records this shop never saw.
 *
 * The owner is one click away in the header — the other half of the round trip that starts on the pets
 * directory.
 */
export function PetDetail() {
  const { id = "" } = useParams();
  const { api } = useApp();
  const navigate = useNavigate();
  const { run, busy } = useAction();

  const [pet, setPet] = useState<CrmPet | null>(null);
  const [appointments, setAppointments] = useState<CrmAppointment[]>([]);
  const [verifications, setVerifications] = useState<CrmVerification[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // Three bounded reads, each independently pageable — mirroring the client detail page. The
      // verification read is PET-scoped: `clientId` would pull in this pet's siblings' checks and
      // present them as this pet's.
      const [p, appts, verifs] = await Promise.all([
        api.getPet(id),
        api.listAppointments({ petId: id, limit: 25 }),
        api.listVerifications({ petId: id, limit: 25 }),
      ]);
      setPet(p);
      setAppointments(appts.rows);
      setVerifications(verifs.rows);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [api, id]);

  useEffect(() => {
    void load();
  }, [load]);

  if (loading && !pet) {
    return (
      <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted">
        <Spinner className="h-4 w-4" /> Loading pet…
      </div>
    );
  }
  if (error || !pet) {
    return (
      <Card>
        <CardContent className="space-y-3 pt-6">
          <p className="text-sm text-danger">{error ?? "Pet not found."}</p>
          <Button variant="outline" size="sm" onClick={() => navigate("/pets")}>
            <ArrowLeft className="h-4 w-4" /> Back to pets
          </Button>
        </CardContent>
      </Card>
    );
  }

  if (editing) {
    return (
      <PetEditForm
        pet={pet}
        onCancel={() => setEditing(false)}
        onSaved={async () => {
          setEditing(false);
          await load();
        }}
      />
    );
  }

  async function remove() {
    const ok = await run(() => api.deletePet(id), {
      success: "Pet deleted",
      // The backend refuses while appointments or verifications exist rather than orphaning them.
      failure: "Could not delete the pet",
    });
    if (ok) navigate("/pets");
  }

  const subtitle = [pet.species, pet.breed, pet.sex, pet.dateOfBirth].filter(Boolean).join(" · ");

  return (
    <div className="space-y-4">
      <Link
        to="/pets"
        className="inline-flex items-center gap-1 text-sm text-muted hover:text-onSurface"
      >
        <ArrowLeft className="h-4 w-4" /> All pets
      </Link>

      <Card>
        <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2">
              <PawPrint className="h-5 w-5 text-primary" /> {pet.name}
            </CardTitle>
            <CardDescription>
              {subtitle && <span>{subtitle} · </span>}
              {/* Pet -> owner. The pets directory is the other direction. */}
              <Link to={`/clients/${pet.clientId}`} className="text-primary hover:underline">
                <User className="mr-0.5 inline h-3.5 w-3.5" />
                {pet.clientName || "owner"}
              </Link>
            </CardDescription>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" onClick={() => setEditing(true)}>
              <Pencil className="h-4 w-4" /> Edit
            </Button>
            <Button
              size="sm"
              onClick={() =>
                navigate(`/appointments/new?clientId=${pet.clientId}&petId=${pet.petId}`)
              }
            >
              <CalendarPlus className="h-4 w-4" /> Book appointment
            </Button>
            <Button variant="ghost" size="sm" loading={busy} onClick={() => void remove()}>
              <Trash2 className="h-4 w-4" /> Delete
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-6">
          <dl className="grid gap-4 text-sm sm:grid-cols-3">
            <Field label="Species" value={pet.species} />
            <Field label="Breed" value={pet.breed} />
            <Field label="Sex" value={pet.sex} />
            <Field label="Date of birth" value={pet.dateOfBirth} />
            {/*
              Rendered as an ordinary detail, with no badge and no nudge when it is absent. A pet
              with no microchip is an ordinary pet — many are not chipped — so a warning here would
              nag the commonest case in the shop. What an absent chip costs is the cross-check when a
              DogTag is linked, and the DogTag card below says so at the moment it matters.
            */}
            <Field label="Microchip" value={pet.microchipCode ?? ""} />
            <Field label="Owner" value={pet.clientName} />
          </dl>
          {pet.notes && (
            <div className="space-y-1">
              <p className="text-xs font-semibold uppercase tracking-wide text-muted">Notes</p>
              <p className="whitespace-pre-wrap text-sm text-onSurface">{pet.notes}</p>
            </div>
          )}
        </CardContent>
      </Card>

      <PetDogTagCard pet={pet} onChanged={load} />

      <PetTagCredentials pet={pet} />

      <TagDiscoveryPanel dogTagId={pet.dogTagId ?? ""} petId={pet.petId} petName={pet.name} />

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Appointments</CardTitle>
          <CardDescription>The most recent bookings for this pet.</CardDescription>
        </CardHeader>
        <CardContent>
          <ListPlaceholder
            loading={loading}
            error={null}
            empty={appointments.length === 0}
            emptyMessage="No appointments booked for this pet yet."
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>When</TableHead>
                  <TableHead>Service</TableHead>
                  <TableHead>Groomer</TableHead>
                  <TableHead>Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {appointments.map((a) => (
                  <TableRow key={a.appointmentId}>
                    <TableCell>
                      <Link
                        to={`/appointments/${a.appointmentId}`}
                        className="text-primary hover:underline"
                      >
                        {formatDateTime(a.startAt)}
                      </Link>
                    </TableCell>
                    <TableCell>{a.service || "—"}</TableCell>
                    <TableCell>{a.groomer || "—"}</TableCell>
                    <TableCell>
                      <AppointmentStatusBadge status={a.status} />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </ListPlaceholder>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <ShieldCheck className="h-4 w-4 text-primary" /> Verifications
          </CardTitle>
          <CardDescription>
            Every check this shop performed on this pet.{" "}
            <Link
              to={`/verifications?clientId=${pet.clientId}`}
              className="text-primary hover:underline"
            >
              See all for this owner
            </Link>
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ListPlaceholder
            loading={loading}
            error={null}
            empty={verifications.length === 0}
            emptyMessage="No verifications recorded for this pet yet."
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>When</TableHead>
                  <TableHead>Purpose</TableHead>
                  <TableHead>Disclosed</TableHead>
                  <TableHead>Result</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {verifications.map((v) => (
                  <TableRow key={v.verificationId}>
                    <TableCell>
                      <Link
                        to={`/verifications/${v.verificationId}`}
                        className="text-primary hover:underline"
                      >
                        {formatDateTime(v.createdAt)}
                      </Link>
                    </TableCell>
                    <TableCell>{v.purpose}</TableCell>
                    <TableCell>
                      <DisclosureBadge disclosedKeyPaths={v.disclosedKeyPaths} />
                    </TableCell>
                    <TableCell>
                      <VerificationStatusBadge status={v.status} />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </ListPlaceholder>
        </CardContent>
      </Card>
    </div>
  );
}

/**
 * The pet's DogTag: LINK and UNLINK, and nothing that touches the chain.
 *
 * The two words are kept apart deliberately and permanently. "Unlink" edits THIS SHOP's record of
 * which tag belongs to which pet — local, private, instantly reversible. "Revoke" is a transaction
 * that invalidates a credential for everyone, forever, in public. They are not variants of one
 * action, so they must never be adjacent buttons with similar labels, and the destructive one is not
 * on this card at all: this deployment has no authority to perform it (see below).
 */
function PetDogTagCard({ pet, onChanged }: { pet: CrmPet; onChanged: () => Promise<void> }) {
  const { api } = useApp();
  const { run, busy } = useAction();
  const { toast } = useToast();
  const [linking, setLinking] = useState(false);
  const [tagInput, setTagInput] = useState("");
  const [confirmingUnlink, setConfirmingUnlink] = useState(false);
  // The microchip cross-check's verdict, from the LAST link attempt. Held on BOTH paths — a refusal
  // carries the same structured object as a success — so the operator sees one explanation whichever
  // way it went, rather than a rendered state on the happy path and a toast on the sad one.
  const [check, setCheck] = useState<MicrochipCheck | null>(null);

  async function link(e: FormEvent) {
    e.preventDefault();
    const tag = tagInput.trim();
    if (!tag) return;
    setCheck(null);
    setLinking(true);
    try {
      const linked = await api.linkPetDogTag(pet.petId, tag);
      setCheck(linked.microchipCheck ?? null);
      toast({ title: `DogTag ${tag} linked to ${pet.name}`, variant: "success" });
      setTagInput("");
      await onChanged();
    } catch (e) {
      const refused = microchipCheckFromError(e);
      setCheck(refused);
      // A microchip refusal explains itself in the panel below, which can name both codes. The toast
      // therefore only announces THAT it was refused; repeating the explanation would be the same
      // sentence twice, and dropping the toast entirely would let a refusal pass unnoticed. Any other
      // failure still carries its message, since the panel has nothing to say about it.
      toast({
        title: refused ? "DogTag not linked" : "Could not link the DogTag",
        description: refused ? "The microchip does not match — see below." : (e as Error).message,
        variant: "danger",
      });
    } finally {
      setLinking(false);
    }
  }

  async function unlink() {
    const ok = await run(() => api.unlinkPetDogTag(pet.petId), {
      success: "DogTag removed from this record",
      failure: "Could not remove the DogTag",
    });
    if (ok) {
      setConfirmingUnlink(false);
      await onChanged();
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Link2 className="h-4 w-4 text-primary" /> DogTag
        </CardTitle>
        <CardDescription>
          Which DogTag this pet holds, as recorded by this shop. This is the id every check and every
          on-chain lookup for {pet.name} is keyed by.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {pet.dogTagId ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline" className="font-mono">
                {pet.dogTagId}
              </Badge>
              <span className="text-xs text-muted">linked to this pet's record</span>
            </div>

            {confirmingUnlink ? (
              <div className="space-y-3 rounded-md border border-border bg-surface-muted p-3">
                <p className="text-sm font-medium text-onSurface">
                  Remove DogTag {pet.dogTagId} from {pet.name}'s record?
                </p>
                <p className="text-sm text-muted">
                  This changes only this shop's record. The credential stays valid, stays on chain, and
                  stays verifiable by everyone else — nothing is revoked and nothing is published. You
                  can link it again at any time. What you lose until then is the ability to check this
                  pet or to look its history up on chain.
                </p>
                <div className="flex flex-wrap gap-2">
                  <Button variant="outline" size="sm" loading={busy} onClick={() => void unlink()}>
                    <Link2Off className="h-4 w-4" /> Remove from this record
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => setConfirmingUnlink(false)}>
                    Keep it
                  </Button>
                </div>
              </div>
            ) : (
              <Button variant="outline" size="sm" onClick={() => setConfirmingUnlink(true)}>
                <Link2Off className="h-4 w-4" /> Remove from this record
              </Button>
            )}
          </>
        ) : (
          <>
            <p className="text-sm text-muted">
              No DogTag linked. Ask the owner to open their DogTag app and read out the tag id, or scan
              it, then add it here.
            </p>
            <form onSubmit={link} className="flex flex-wrap items-end gap-2">
              <div className="min-w-[16rem] flex-1 space-y-2">
                <Label htmlFor="pet-dogtag">DogTag id</Label>
                <Input
                  id="pet-dogtag"
                  value={tagInput}
                  onChange={(e) => setTagInput(e.target.value)}
                  placeholder="The number the owner's app shows, or the full on-chain id"
                />
              </div>
              <Button type="submit" size="sm" loading={linking} disabled={!tagInput.trim()}>
                <Link2 className="h-4 w-4" /> Add DogTag
              </Button>
            </form>
          </>
        )}

        {check && <MicrochipCheckPanel check={check} />}

        {/*
          Why there is no Issue and no Revoke button here. Hiding the absence would leave an operator
          hunting for a control that does not exist; stating it names the reason, which is the same
          reason their own verification means anything.
        */}
        <div className="space-y-1 rounded-md border border-dashed border-border p-3 text-xs text-muted">
          <p className="font-semibold uppercase tracking-wide">Issuing and revoking</p>
          <p>
            This portal cannot mint a DogTag or revoke a credential. A groomer is whitelisted to VERIFY,
            not to issue — which is exactly what makes a vaccination check here worth anything, since
            the shop asserting the record cannot also be the shop that wrote it. Ask the issuing vet or
            authority to issue or revoke; adding and removing the tag above only ever edits this shop's
            own record.
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

function PetEditForm({
  pet,
  onCancel,
  onSaved,
}: {
  pet: CrmPet;
  onCancel: () => void;
  onSaved: () => Promise<void>;
}) {
  const { api } = useApp();
  const { run, busy } = useAction();
  const [draft, setDraft] = useState<ClientPetInput>({
    name: pet.name,
    species: pet.species,
    breed: pet.breed,
    sex: pet.sex,
    dateOfBirth: pet.dateOfBirth,
    notes: pet.notes,
    microchipCode: pet.microchipCode ?? "",
  });

  const nameInvalid = draft.name.trim().length === 0;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (nameInvalid) return;
    // The DogTag id is deliberately absent from this body: an edit to a pet's details must not be able
    // to change which credential it is bound to.
    const ok = await run(
      () =>
        api.updatePet(pet.petId, {
          name: draft.name,
          species: draft.species,
          breed: draft.breed,
          sex: draft.sex,
          dateOfBirth: draft.dateOfBirth,
          notes: draft.notes,
          // Sent even when blank, because an explicit blank CLEARS the code — which is how an
          // operator corrects a microchip typed onto the wrong pet. A wrong code that cannot be
          // cleared would refuse every future link with no way out.
          microchipCode: draft.microchipCode ?? "",
        }),
      { success: "Pet updated", failure: "Could not save the pet" },
    );
    if (ok) await onSaved();
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Edit {pet.name}</CardTitle>
        <CardDescription>
          Owner: {pet.clientName}. Moving a pet to a different owner is not an edit — create it under
          the new owner instead, so its booking history stays with the person it happened under.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="space-y-4">
          <PetFields
            value={draft}
            idPrefix={`edit-pet-${pet.petId}`}
            onChange={(patch) => setDraft((d) => ({ ...d, ...patch }))}
          />
          <div className="flex flex-wrap gap-2">
            <Button type="submit" loading={busy} disabled={nameInvalid}>
              Save changes
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

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-xs font-semibold uppercase tracking-wide text-muted">{label}</dt>
      <dd className="mt-1 text-onSurface">{value || "—"}</dd>
    </div>
  );
}

/**
 * The microchip cross-check's verdict, rendered in every state.
 *
 * A DogTag link is otherwise unchecked — a mistyped digit or two similar dogs in one afternoon give
 * a link that is structurally perfect and about the wrong animal. The credential carries the
 * microchip as a Merkle leaf, so when the shop has one on file too the two can be compared, and this
 * panel says what that comparison found.
 *
 * The whole point is the middle state. "Not compared" is neither a pass nor a failure: it is
 * rendered with its own wording, in its own tone, saying which side was missing or which read did
 * not resolve. Most often that is simply "this pet has no microchip on file" — an ordinary fact
 * about an ordinary animal, so it stays NEUTRAL and does not nag. A failure to LOOK (an unreadable
 * store, a document that will not parse) warns instead, because something that should have resolved
 * did not. Both come from `microchipTone`, which reads the backend's own fact/failure flag rather
 * than re-deriving it here.
 */
function MicrochipCheckPanel({ check }: { check: MicrochipCheck }) {
  const tone = microchipTone(check);
  const style =
    tone === "positive"
      ? "border-success/40 bg-success/10 text-onSurface"
      : tone === "negative"
        ? "border-danger/40 bg-danger/10 text-onSurface"
        : tone === "warning"
          ? "border-warning/40 bg-warning/10 text-onSurface"
          : "border-border bg-surface-muted text-onSurface";
  return (
    <div className={`space-y-1 rounded-md border p-3 text-sm ${style}`} data-testid="microchip-check">
      <p className="font-semibold" data-testid="microchip-check-headline">
        {microchipHeadline(check)}
      </p>
      <p className="text-muted">{microchipExplanation(check)}</p>
    </div>
  );
}
