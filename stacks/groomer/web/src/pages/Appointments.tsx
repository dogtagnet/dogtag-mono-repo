import { ListChecks } from "lucide-react";
import { Placeholder } from "./Placeholder";

export function Appointments() {
  return (
    <Placeholder
      icon={ListChecks}
      title="Appointments"
      description="Approve / decline / reschedule appointments synced from the central backend (§4.4)."
    />
  );
}
