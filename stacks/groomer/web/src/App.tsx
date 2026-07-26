import { Navigate, Route, Routes } from "react-router-dom";
import { useApp } from "./app/AppContext";
import { Layout } from "./app/Layout";
import { Login } from "./pages/Login";
import { Dashboard } from "./pages/Dashboard";
import { Calendar } from "./pages/Calendar";
import { Appointments } from "./pages/Appointments";
import { AppointmentDetail } from "./pages/AppointmentDetail";
import { AppointmentForm } from "./pages/AppointmentForm";
import { Clients } from "./pages/Clients";
import { ClientDetail } from "./pages/ClientDetail";
import { Groomers } from "./pages/Groomers";
import { Reports } from "./pages/Reports";
import { Marketing } from "./pages/Marketing";
import { ImportFromUser } from "./pages/ImportFromUser";
import { Verifications } from "./pages/Verifications";
import { VerificationDetail } from "./pages/VerificationDetail";
import { Verify } from "./pages/Verify";
import { Setup } from "./pages/Setup";
import { Settings } from "./pages/Settings";
import { Unlock } from "./pages/Unlock";

/**
 * A groomer VERIFIES; it does not ISSUE. There is deliberately no `/issue` route here — and the
 * backend does not mount the issuance endpoints for `BUSINESS_TYPE=groomer` either, so the portal
 * and the API agree on what this deployment is.
 */
export function App() {
  const { opToken } = useApp();

  if (!opToken) return <Login />;

  return (
    <Routes>
      {/* Standalone (no Layout): unlocking is the whole page, not a section of the portal. */}
      <Route path="/unlock" element={<Unlock />} />
      <Route path="/dashboard" element={<Layout title="Dashboard"><Dashboard /></Layout>} />
      <Route path="/calendar" element={<Layout title="Calendar"><Calendar /></Layout>} />
      {/* `/new` is declared before `/:id` so it is matched as the create form, not an id. */}
      <Route
        path="/appointments/new"
        element={<Layout title="New appointment"><AppointmentForm mode="create" /></Layout>}
      />
      <Route
        path="/appointments/:id/edit"
        element={<Layout title="Edit appointment"><AppointmentForm mode="edit" /></Layout>}
      />
      <Route path="/appointments/:id" element={<Layout title="Appointment"><AppointmentDetail /></Layout>} />
      <Route path="/appointments" element={<Layout title="Appointments"><Appointments /></Layout>} />
      <Route path="/clients/:id" element={<Layout title="Client"><ClientDetail /></Layout>} />
      <Route path="/clients" element={<Layout title="Clients"><Clients /></Layout>} />
      <Route path="/verifications/:id" element={<Layout title="Verification"><VerificationDetail /></Layout>} />
      <Route path="/verifications" element={<Layout title="All verifications"><Verifications /></Layout>} />
      <Route path="/groomers" element={<Layout title="Groomers"><Groomers /></Layout>} />
      <Route path="/reports" element={<Layout title="Reports"><Reports /></Layout>} />
      <Route path="/marketing" element={<Layout title="Marketing"><Marketing /></Layout>} />
      <Route path="/import" element={<Layout title="Import from user"><ImportFromUser /></Layout>} />
      <Route path="/verify" element={<Layout title="Ad-hoc verification"><Verify /></Layout>} />
      <Route path="/setup" element={<Layout title="Setup"><Setup /></Layout>} />
      <Route path="/settings" element={<Layout title="Settings"><Settings /></Layout>} />
      <Route path="*" element={<Navigate to="/dashboard" replace />} />
    </Routes>
  );
}
