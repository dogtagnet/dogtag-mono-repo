import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@dogtag/ui";
import { Download, ShieldCheck, Wand2 } from "lucide-react";
import type { ComponentType } from "react";
import { Link } from "react-router-dom";

const QUICK_LINKS: {
  href: string;
  label: string;
  blurb: string;
  icon: ComponentType<{ className?: string }>;
}[] = [
  {
    href: "/import",
    label: "Import from customer",
    blurb: "Pull a pet profile / vaccination via QR and verify on chain + DNS before accepting.",
    icon: Download,
  },
  {
    href: "/verify",
    label: "Export on chain",
    blurb: "The owner exports a proof; record a proof-of-verification (Normal/ZK) without being a credential issuer.",
    icon: ShieldCheck,
  },
  {
    href: "/setup",
    label: "Custody setup",
    blurb: "Genesis + unlock the server key so the shop can issue its own records.",
    icon: Wand2,
  },
];

export function Dashboard() {
  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Welcome to your groomer portal</CardTitle>
          <CardDescription>
            Manage appointments and clients, and use the DogTag flows to import, verify and issue pet
            credentials on ROAX.
          </CardDescription>
        </CardHeader>
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
    </div>
  );
}
