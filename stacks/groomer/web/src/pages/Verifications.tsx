import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@dogtag/ui";
import type { CrmVerification } from "@dogtag/ui";
import { History, Search } from "lucide-react";
import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useApp } from "../app/AppContext";
import {
  ListPlaceholder,
  DisclosureBadge,
  PAGE_SIZE,
  Pager,
  VerificationStatusBadge,
  useDebounced,
  useList,
} from "../app/crm";
import { formatDateTime, startOfDayFromInput } from "../lib/time";
import { VERIFY_PURPOSES } from "./verifyPurposes";

const ANY = "any";

const STATUSES: CrmVerification["status"][] = ["pending", "recording", "recorded", "error"];

/**
 * "All verifications" — the shop's complete, searchable history of every verification it has
 * performed, joined to the appointment and client where the operator started it from one.
 *
 * `?clientId=` / `?appointmentId=` in the URL pre-apply that filter, so the client and appointment
 * pages can deep-link straight into a scoped history.
 */
export function Verifications() {
  const { api } = useApp();
  const [params, setParams] = useSearchParams();

  const clientId = params.get("clientId") ?? "";
  const appointmentId = params.get("appointmentId") ?? "";

  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<CrmVerification["status"] | typeof ANY>(ANY);
  const [purpose, setPurpose] = useState<string>(ANY);
  const [fromDate, setFromDate] = useState("");
  const [toDate, setToDate] = useState("");
  const [offset, setOffset] = useState(0);
  const q = useDebounced(search);

  // inclusive day inputs -> the backend's half-open [from, to) window
  const from = fromDate ? startOfDayFromInput(fromDate) : undefined;
  const to = toDate ? startOfDayFromInput(toDate) + 86_400 : undefined;

  const { page, loading, error } = useList<CrmVerification>(
    () =>
      api.listVerifications({
        q,
        clientId: clientId || undefined,
        appointmentId: appointmentId || undefined,
        status: status === ANY ? undefined : status,
        purpose: purpose === ANY ? undefined : purpose,
        from,
        to,
        limit: PAGE_SIZE,
        offset,
      }),
    [q, clientId, appointmentId, status, purpose, from, to, offset],
  );

  function set<T>(setter: (v: T) => void) {
    return (v: T) => {
      setter(v);
      setOffset(0);
    };
  }

  const rows = page?.rows ?? [];
  const scoped = clientId !== "" || appointmentId !== "";
  const filtered =
    scoped || q !== "" || status !== ANY || purpose !== ANY || !!from || !!to;

  function clearAll() {
    setSearch("");
    setStatus(ANY);
    setPurpose(ANY);
    setFromDate("");
    setToDate("");
    setOffset(0);
    setParams({});
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <History className="h-5 w-5 text-primary" /> All verifications
        </CardTitle>
        <CardDescription>
          Every proof-of-verification this shop has recorded, with its on-chain evidence. Filter by
          client, appointment, purpose, result or date.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {scoped && (
          <p className="rounded-md border border-border bg-surface-muted px-3 py-2 text-sm">
            Showing verifications for{" "}
            {clientId ? (
              <Link to={`/clients/${clientId}`} className="text-primary hover:underline">
                one client
              </Link>
            ) : (
              <Link to={`/appointments/${appointmentId}`} className="text-primary hover:underline">
                one appointment
              </Link>
            )}
            .
          </p>
        )}

        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <div className="relative lg:col-span-2">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
            <Input
              className="pl-9"
              placeholder="Search client, pet, purpose, tx or DogTag id…"
              value={search}
              onChange={(e) => set(setSearch)(e.target.value)}
              aria-label="Search verifications"
            />
          </div>
          <Select
            value={status}
            onValueChange={(v) => set(setStatus)(v as CrmVerification["status"] | typeof ANY)}
          >
            <SelectTrigger aria-label="Filter by result">
              <SelectValue placeholder="Any result" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ANY}>Any result</SelectItem>
              {STATUSES.map((s) => (
                <SelectItem key={s} value={s}>
                  {s}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select value={purpose} onValueChange={set(setPurpose)}>
            <SelectTrigger aria-label="Filter by purpose">
              <SelectValue placeholder="Any purpose" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ANY}>Any purpose</SelectItem>
              {VERIFY_PURPOSES.map((p) => (
                <SelectItem key={p.value} value={p.value}>
                  {p.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="flex items-end gap-2 lg:col-span-2">
            <div className="flex-1 space-y-1">
              <Label htmlFor="verif-from" className="text-xs">
                From
              </Label>
              <Input
                id="verif-from"
                type="date"
                value={fromDate}
                onChange={(e) => set(setFromDate)(e.target.value)}
              />
            </div>
            <div className="flex-1 space-y-1">
              <Label htmlFor="verif-to" className="text-xs">
                To
              </Label>
              <Input
                id="verif-to"
                type="date"
                value={toDate}
                onChange={(e) => set(setToDate)(e.target.value)}
              />
            </div>
          </div>
        </div>

        {filtered && (
          <Button variant="ghost" size="sm" onClick={clearAll}>
            Clear filters
          </Button>
        )}

        <ListPlaceholder
          loading={loading}
          error={error}
          empty={rows.length === 0}
          emptyMessage={
            filtered
              ? "No verifications match these filters."
              : "No verifications recorded yet — start one from an appointment."
          }
        >
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>When</TableHead>
                <TableHead>Client</TableHead>
                <TableHead>Pet</TableHead>
                <TableHead>Purpose</TableHead>
                <TableHead>Disclosed</TableHead>
                <TableHead>Result</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((v) => (
                <TableRow key={v.verificationId}>
                  <TableCell className="whitespace-nowrap">
                    <Link
                      to={`/verifications/${v.verificationId}`}
                      className="text-primary hover:underline"
                    >
                      {formatDateTime(v.createdAt)}
                    </Link>
                  </TableCell>
                  <TableCell>
                    {v.clientId ? (
                      <Link to={`/clients/${v.clientId}`} className="hover:underline">
                        {v.clientName || "—"}
                      </Link>
                    ) : (
                      <span className="text-muted">Ad-hoc</span>
                    )}
                  </TableCell>
                  <TableCell>{v.petName || "—"}</TableCell>
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

        <Pager
          total={page?.total ?? 0}
          offset={offset}
          limit={page?.limit ?? PAGE_SIZE}
          onOffset={setOffset}
        />
      </CardContent>
    </Card>
  );
}
