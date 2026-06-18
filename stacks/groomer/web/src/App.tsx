import { Navigate, Route, Routes } from "react-router-dom";
import { useApp } from "./app/AppContext";
import { Layout } from "./app/Layout";
import { Login } from "./pages/Login";
import { Dashboard } from "./pages/Dashboard";
import { Calendar } from "./pages/Calendar";
import { Appointments } from "./pages/Appointments";
import { Clients } from "./pages/Clients";
import { Groomers } from "./pages/Groomers";
import { Reports } from "./pages/Reports";
import { Marketing } from "./pages/Marketing";
import { ImportFromUser } from "./pages/ImportFromUser";
import { Issue } from "./pages/Issue";
import { Verify } from "./pages/Verify";
import { Setup } from "./pages/Setup";
import { Settings } from "./pages/Settings";

export function App() {
  const { opToken } = useApp();

  if (!opToken) return <Login />;

  return (
    <Routes>
      <Route path="/dashboard" element={<Layout title="Dashboard"><Dashboard /></Layout>} />
      <Route path="/calendar" element={<Layout title="Calendar"><Calendar /></Layout>} />
      <Route path="/appointments" element={<Layout title="Appointments"><Appointments /></Layout>} />
      <Route path="/clients" element={<Layout title="Clients"><Clients /></Layout>} />
      <Route path="/groomers" element={<Layout title="Groomers"><Groomers /></Layout>} />
      <Route path="/reports" element={<Layout title="Reports"><Reports /></Layout>} />
      <Route path="/marketing" element={<Layout title="Marketing"><Marketing /></Layout>} />
      <Route path="/import" element={<Layout title="Import from user"><ImportFromUser /></Layout>} />
      <Route path="/issue" element={<Layout title="Issue credential"><Issue /></Layout>} />
      <Route path="/verify" element={<Layout title="Verify"><Verify /></Layout>} />
      <Route path="/setup" element={<Layout title="Setup"><Setup /></Layout>} />
      <Route path="/settings" element={<Layout title="Settings"><Settings /></Layout>} />
      <Route path="*" element={<Navigate to="/dashboard" replace />} />
    </Routes>
  );
}
