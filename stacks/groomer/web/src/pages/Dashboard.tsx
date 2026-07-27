import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Spinner,
} from "@dogtag/ui";
import type { CrmAppointment } from "@dogtag/ui";
import { CalendarDays, History, ShieldCheck, Users, Wand2 } from "lucide-react";
import type { ComponentType } from "react";
import { Link } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { AppointmentStatusBadge, ListPlaceholder, useList } from "../app/crm";
import { DAY_SECS, formatSlot, nowSec, startOfDay } from "../lib/time";

const QUICK_LINKS: {
  href: string;
  label: string;
  blurb: string;
  icon: ComponentType<{ className?: string }>;
}[] = [
  {
    href: "/calendar",
    label: "Calendar",
    blurb: "Your day and week at a glance — every booking in one grid.",
    icon: CalendarDays,
  },
  {
    href: "/clients",
    label: "Clients",
    blurb: "Customer directory and their pets. Book an appointment from a client.",
    icon: Users,
  },
  {
    href: "/verifications",
    label: "All verifications",
    blurb: "Every vaccination check you have run, searchable by client and appointment.",
    icon: History,
  },
  {
    // The appointment-linked flow is the primary one; this is the walk-in path, deliberately kept.
    href: "/verify",
    label: "Ad-hoc verification",
    blurb:
      "A walk-in with no booking: the owner approves an owner-hidden proof, recorded without becoming a credential issuer.",
    icon: ShieldCheck,
  },
  {
    href: "/setup",
    label: "Custody setup",
    blurb:
      "Genesis the server key that relays your verifications on chain. Unlocking has its own page.",
    icon: Wand2,
  },
];

/**
 * The shop's landing surface: today's bookings first, then the sections an operator reaches for.
 * A groomer VERIFIES — there is deliberately no issuance here.
 */
export function Dashboard() {
  const { api } = useApp();
  const todayStart = startOfDay(nowSec());

  const { page, loading, error } = useList<CrmAppointment>(
    () => api.listAppointments({ from: todayStart, to: todayStart + DAY_SECS, limit: 50 }),
    [todayStart],
  );

  const today = page?.rows ?? [];

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2">
              <CalendarDays className="h-5 w-5 text-primary" /> Today
            </CardTitle>
            <CardDescription>
              {loading && today.length === 0
                ? "Loading today's bookings…"
                : `${today.length} ${today.length === 1 ? "booking" : "bookings"} today.`}
            </CardDescription>
          </div>
          {loading && today.length > 0 && <Spinner className="h-4 w-4 text-muted" />}
        </CardHeader>
        <CardContent>
          <ListPlaceholder
            loading={loading}
            error={error}
            empty={today.length === 0}
            emptyMessage="Nothing booked for today."
          >
            <ul className="divide-y divide-border">
              {today.map((a) => (
                <li key={a.appointmentId}>
                  <Link
                    to={`/appointments/${a.appointmentId}`}
                    className="flex flex-wrap items-center justify-between gap-3 py-3 transition-colors hover:bg-surface-muted/50"
                  >
                    <div className="min-w-0">
                      <p className="truncate font-medium text-onSurface">
                        {formatSlot(a.startAt, a.endAt)} · {a.clientName}
                      </p>
                      <p className="truncate text-sm text-muted">
                        {[a.petName, a.service].filter(Boolean).join(" · ") || "—"}
                      </p>
                    </div>
                    <AppointmentStatusBadge status={a.status} />
                  </Link>
                </li>
              ))}
            </ul>
          </ListPlaceholder>
        </CardContent>
      </Card>

      <div className="grid gap-4 sm:grid-cols-3">
        {QUICK_LINKS.map((q) => {
          const Icon = q.icon;
          return (
            <Link key={q.href} to={q.href} className="block">
              <Card className="h-full transition-colors hover:bg-surface-muted">
                <CardContent className="space-y-2 pt-6">
                  <Icon className="h-6 w-6 text-primary" />
                  <p className="font-medium text-onSurface">{q.label}</p>
                  <p className="text-sm text-muted">{q.blurb}</p>
                </CardContent>
              </Card>
            </Link>
          );
        })}
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <ShieldCheck className="h-4 w-4 text-primary" /> How verification works here
          </CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted">
          Open an appointment and start the check from it. The owner approves on their phone, and the
          result is recorded against that appointment and client. The owner proves their pet's
          vaccination is valid without revealing anything about themselves, and chooses for
          themselves whether to disclose any further details — you get a verifiable answer, not
          their data.
        </CardContent>
      </Card>
    </div>
  );
}
