import { CalendarDays } from "lucide-react";
import { Placeholder } from "./Placeholder";

export function Calendar() {
  return (
    <Placeholder
      icon={CalendarDays}
      title="Calendar"
      description="Connect Google Calendar and view a grid of bookings (mirrors the reference groomer UI)."
    />
  );
}
