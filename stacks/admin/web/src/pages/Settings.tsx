import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  RpcEndpointSettingsCard,
  ThemeToggle,
} from "@dogtag/ui";
import { env } from "../lib/env";

export function Settings() {
  return (
    <div className="space-y-6">
      <RpcEndpointSettingsCard defaultRpcUrl={env.roaxRpc} />

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
