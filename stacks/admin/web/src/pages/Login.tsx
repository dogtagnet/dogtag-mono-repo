import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  ThemeToggle,
  useToast,
  DEMO_ADMIN_PASSWORD,
} from "@dogtag/ui";
import { useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";

/** Admin session gate — POST /v1/admin/login on the central backend. */
export function Login() {
  const { central, setAdminToken } = useApp();
  const { toast } = useToast();
  // Testnet demo: prefill the admin password so the operator just clicks Sign in.
  const [password, setPassword] = useState(DEMO_ADMIN_PASSWORD);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await central.adminLogin(password);
      setAdminToken(r.token);
      toast({ title: "Signed in", variant: "success" });
    } catch (err) {
      toast({ title: "Login failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background px-4">
      <div className="absolute right-4 top-4">
        <ThemeToggle />
      </div>
      <Card className="w-full max-w-sm">
        <CardHeader>
          <div className="mb-2 flex items-center gap-2">
            <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary font-bold text-onPrimary">
              DT
            </span>
            <span className="text-sm font-semibold uppercase tracking-wide text-muted">
              Admin Portal
            </span>
          </div>
          <CardTitle>Admin sign in</CardTitle>
          <CardDescription>Enter the central admin password to continue.</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={submit} className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="pw" required>
                Admin password
              </Label>
              <Input
                id="pw"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoFocus
                required
              />
              <p className="text-xs text-muted">Demo default prefilled — just click Sign in.</p>
            </div>
            <Button type="submit" className="w-full" loading={busy}>
              Sign in
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
