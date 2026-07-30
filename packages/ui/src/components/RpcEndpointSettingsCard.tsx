import { AlertTriangle, RotateCcw, Save } from "lucide-react";
import type { FormEvent } from "react";
import { useRoaxRpcSettings } from "../chain/useRoaxRpcSettings";
import { Badge } from "./Badge";
import { Button } from "./Button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "./Card";
import { Input } from "./Input";
import { Label } from "./Label";

export interface RpcEndpointSettingsCardProps {
  /** The deployment's bundled ROAX endpoint (normally `VITE_ROAX_RPC`). */
  defaultRpcUrl?: string;
}

/**
 * Shared settings surface for shipped browser portals.
 *
 * Saving validates `eth_chainId` first. A different-chain endpoint is never persisted and never
 * receives an address-bound request; reads continue through the independently guarded bundled
 * default.
 */
export function RpcEndpointSettingsCard({
  defaultRpcUrl,
}: RpcEndpointSettingsCardProps) {
  const settings = useRoaxRpcSettings(defaultRpcUrl);

  function save(event: FormEvent) {
    event.preventDefault();
    void settings.save();
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Blockchain endpoint</CardTitle>
        <CardDescription>
          Choose the JSON-RPC peer this browser uses for gasless ROAX reads. A custom endpoint can
          improve availability and help when one endpoint censors requests.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <form onSubmit={save} className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="roax-rpc-url">ROAX JSON-RPC URL</Label>
            <Input
              id="roax-rpc-url"
              type="url"
              inputMode="url"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              value={settings.draft}
              disabled={settings.checking}
              invalid={settings.message?.tone === "danger"}
              onChange={(event) => settings.setDraft(event.target.value)}
              placeholder="https://rpc.example"
            />
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button type="submit" loading={settings.checking}>
              <Save className="h-4 w-4" />
              Check and save
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={settings.reset}
              disabled={
                settings.checking ||
                (!settings.isCustom && settings.draft === settings.defaultRpcUrl)
              }
            >
              <RotateCcw className="h-4 w-4" />
              Restore default
            </Button>
            <Badge variant={settings.isCustom ? "warning" : "neutral"}>
              {settings.isCustom ? "Custom endpoint" : "Bundled default"}
            </Badge>
          </div>
        </form>

        {settings.message && (
          <p
            role={settings.message.tone === "danger" ? "alert" : "status"}
            className={
              settings.message.tone === "danger"
                ? "text-sm text-danger"
                : "text-sm text-success"
            }
          >
            {settings.message.text}
          </p>
        )}

        <div className="rounded-md border border-warning/40 bg-warning/10 p-3 text-sm text-onSurface">
          <p className="flex items-start gap-2 font-semibold">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            Endpoint choice is not a trust upgrade
          </p>
          <p className="mt-1.5 text-muted">
            This is plain JSON-RPC, not a light client. Any endpoint can fabricate{" "}
            <code>isValid</code>, <code>rootIssuer</code>, <code>profileRoot</code>, logs, or
            transaction data. The chain guard only prevents DogTag from using its bundled contract
            addresses on a different chain.
          </p>
        </div>

        <p className="text-sm text-muted">
          Centralized app APIs and the provider directory/indexer stay fixed and are not changed by
          this setting. Transactions sent through an injected or WalletConnect wallet still use that
          wallet&apos;s own provider; this setting controls the app&apos;s direct blockchain reads.
        </p>
        <p className="break-all text-xs text-muted">
          Bundled default: <code>{settings.defaultRpcUrl}</code>
        </p>
      </CardContent>
    </Card>
  );
}
