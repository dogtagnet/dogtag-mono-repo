import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  SigningModeToggle,
  StatusPanel,
  ThemeToggle,
  useToast,
  type SigningMode,
  type WhitelistRow,
} from "@dogtag/ui";
import { useEffect, useState } from "react";
import { useApp } from "../app/AppContext";

export function Settings() {
  const { api, signingMode, setSigningMode, unlocked } = useApp();
  const { toast } = useToast();
  const [whitelist, setWhitelist] = useState<WhitelistRow[]>([]);
  const [backendSigner, setBackendSigner] = useState<string>();

  // load the whitelist matrix + active signer for the status panel.
  useEffect(() => {
    let cancelled = false;
    api
      .issuerSigners()
      .then((r) => {
        if (cancelled) return;
        setWhitelist(r.matrix);
        setBackendSigner(r.activeSigner || undefined);
      })
      .catch(() => {
        /* not unlocked / unauthenticated */
      });
    return () => {
      cancelled = true;
    };
  }, [api, unlocked]);

  async function changeMode(mode: SigningMode) {
    try {
      const r = await api.putSigningMode(mode);
      setSigningMode(r.signingMode);
      toast({ title: `Signing mode → ${r.signingMode}`, variant: "success" });
    } catch (err) {
      toast({ title: "Could not switch mode", description: (err as Error).message, variant: "danger" });
    }
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Signing mode</CardTitle>
          <CardDescription>
            Browser wallet: you pay PLASMA gas. Server key: the clinic's wallet pays. Persisted
            server-side. Switching is blocked while a prepared record is outstanding.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <SigningModeToggle value={signingMode} onChange={changeMode} />
        </CardContent>
      </Card>

      <StatusPanel
        mode={signingMode}
        whitelist={whitelist}
        genesisState={unlocked ? "initialized" : "locked"}
        backendSignerAddress={signingMode === "backend" ? backendSigner : undefined}
      />

      <Card>
        <CardHeader>
          <CardTitle>Appearance</CardTitle>
          <CardDescription>Toggle light / dark theme (persisted).</CardDescription>
        </CardHeader>
        <CardContent>
          <ThemeToggle />
        </CardContent>
      </Card>
    </div>
  );
}
