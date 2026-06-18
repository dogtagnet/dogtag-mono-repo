import {
  Badge,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Spinner,
} from "@dogtag/ui";
import { Activity, Building2, ListChecks } from "lucide-react";
import { useEffect, useState } from "react";
import { useApp } from "../app/AppContext";

/**
 * Admin dashboard (impl §5.3). Shows live registry / application counts from the central backend;
 * appointment + observability panels are placeholders.
 */
export function Dashboard() {
  const { central } = useApp();
  const [businesses, setBusinesses] = useState<number | null>(null);
  const [pending, setPending] = useState<number | null>(null);
  const [approved, setApproved] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const b = await central.listBusinesses();
        if (!cancelled) setBusinesses(b.businesses.length);
      } catch {
        if (!cancelled) setBusinesses(0);
      }
      try {
        const a = await central.listApplications();
        if (!cancelled) {
          setPending(a.applications.filter((x) => x.status === "pending").length);
          setApproved(a.applications.filter((x) => x.status === "approved").length);
        }
      } catch {
        if (!cancelled) {
          setPending(0);
          setApproved(0);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [central]);

  return (
    <div className="space-y-6">
      <div className="grid gap-4 sm:grid-cols-3">
        <Stat icon={Building2} label="Registered businesses" value={businesses} />
        <Stat icon={ListChecks} label="Pending applications" value={pending} />
        <Stat icon={ListChecks} label="Approved issuers" value={approved} />
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Activity className="h-5 w-5 text-primary" /> Appointments &amp; observability
            <Badge variant="neutral">Placeholder</Badge>
          </CardTitle>
          <CardDescription>
            Appointment throughput and system observability dashboards live here (placeholder in this
            build). The central backend exposes owner-scoped appointment/consent endpoints; an
            aggregate admin view is future work.
          </CardDescription>
        </CardHeader>
        <CardContent className="text-sm text-muted">No metrics wired yet.</CardContent>
      </Card>
    </div>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Building2;
  label: string;
  value: number | null;
}) {
  return (
    <Card>
      <CardContent className="flex items-center gap-4 pt-6">
        <span className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10">
          <Icon className="h-5 w-5 text-primary" />
        </span>
        <div>
          <div className="text-2xl font-semibold text-onSurface">
            {value === null ? <Spinner className="h-5 w-5 text-muted" /> : value}
          </div>
          <div className="text-sm text-muted">{label}</div>
        </div>
      </CardContent>
    </Card>
  );
}
