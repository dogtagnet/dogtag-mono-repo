import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  bindingLine,
  type DnsConfirmationRequired,
  type IssuerDomainBindingState,
} from "@dogtag/ui";

/** A pending confirmation: the live DNS observation was not verified, and the admin must decide. */
export type DnsPrompt = DnsConfirmationRequired & { applicationId: string };

/**
 * The advisory-DNS confirmation, shared by EVERY approve call site (the applications queue and the
 * onboarding wizard).
 *
 * It lives here rather than beside one of them because the 409 is raised by the backend for all of
 * them: a call site without this dialog does not degrade gracefully, it dead-ends on an opaque error
 * with no way forward, which is a defect however correct the 409 is.
 *
 * Copy discipline (captain's rule): state the OBSERVATION, never a verdict. A missing TXT record means
 * the domain owner has not published the binding — it says nothing about the organisation, whose
 * legitimacy is the accreditation review's business, not DNS's.
 */
export function DnsConfirmDialog({
  prompt,
  busy,
  onCancel,
  onProceed,
}: {
  prompt: DnsPrompt | null;
  busy: boolean;
  onCancel: () => void;
  onProceed: () => void;
}) {
  if (!prompt) return null;
  const observation = bindingLine({
    state: prompt.dnsState as IssuerDomainBindingState,
    domain: prompt.domain,
  });

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Domain record not confirmed</DialogTitle>
          <DialogDescription>
            You can whitelist this issuer anyway. The outcome below is recorded either way.
          </DialogDescription>
        </DialogHeader>

        <div data-testid="dns-confirm-observation" className="rounded-md border border-border bg-surface-muted p-3">
          <p className="text-sm text-onSurface">{observation}</p>
          <p className="mt-2 text-xs text-muted">
            For the record to be found, <code className="font-mono">{prompt.domain}</code> must publish a
            TXT record with the value{" "}
            <code className="font-mono break-all">{prompt.expectedTxt}</code>.
          </p>
          <p className="mt-2 text-xs text-muted">
            This check shows only whether the domain owner has published that record. It is not a check
            of the organisation, which is what the accreditation review covers.
          </p>
        </div>

        <div className="flex flex-wrap justify-end gap-2">
          <Button variant="outline" onClick={onCancel} disabled={busy}>
            Not now
          </Button>
          <Button data-testid="dns-confirm-proceed" loading={busy} onClick={onProceed}>
            Whitelist anyway
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
