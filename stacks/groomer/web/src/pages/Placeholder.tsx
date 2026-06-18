import {
  Badge,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@dogtag/ui";
import type { ComponentType, ReactNode } from "react";

/**
 * Shared placeholder shell for the reference dashboard sections that are not yet wired to a
 * backend. The DogTag-specific flows (Import / Verify / Setup / Settings) are fully realized;
 * these mirror the reference groomer nav so the shell is complete.
 */
export function Placeholder({
  icon: Icon,
  title,
  description,
  children,
}: {
  icon: ComponentType<{ className?: string }>;
  title: string;
  description: string;
  children?: ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Icon className="h-5 w-5 text-primary" /> {title}
          <Badge variant="neutral">Placeholder</Badge>
        </CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent className="text-sm text-muted">
        {children ?? (
          <p>
            This section mirrors the reference groomer dashboard. It is intentionally a placeholder
            in this build — the realized DogTag flows are Import, Verify, Setup and Settings.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
