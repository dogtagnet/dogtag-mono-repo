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
  DEMO_OPERATOR_PASSWORD,
} from "@dogtag/ui";
import { useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

/** Operator login gate. The admin (custody) session is obtained separately in the Setup wizard. */
export function Login() {
  const { api, setOpToken } = useApp();
  const { toast } = useToast();
  // Demo mode prefills the operator password so the operator just clicks Sign in; in production
  // (VITE_DEMO_MODE unset) the field starts empty and must be typed.
  const [password, setPassword] = useState(env.demoMode ? DEMO_OPERATOR_PASSWORD : "");
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await api.login(password);
      setOpToken(r.token);
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
            <span className="text-sm font-semibold uppercase tracking-wide text-muted">Vet Portal</span>
          </div>
          <CardTitle>Sign in</CardTitle>
          <CardDescription>Enter your operator password to continue.</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={submit} className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="pw" required>
                Operator password
              </Label>
              <Input
                id="pw"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoFocus
                required
              />
              {env.demoMode && (
                <p className="text-xs text-muted">Demo default prefilled — just click Sign in.</p>
              )}
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
