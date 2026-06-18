import { KeyRound, Wallet } from "lucide-react";
import { useState } from "react";
import { cn } from "../lib/cn";
import { Spinner } from "../components/Spinner";
import type { SigningMode } from "../api/types";

interface Option {
  value: SigningMode;
  title: string;
  icon: typeof Wallet;
  blurb: string;
}

const OPTIONS: Option[] = [
  {
    value: "wallet",
    title: "Browser wallet",
    icon: Wallet,
    blurb: "You pay PLASMA gas. MetaMask / WalletConnect signs each issuance.",
  },
  {
    value: "backend",
    title: "Server-managed key",
    icon: KeyRound,
    blurb: "The clinic's wallet pays. The unlocked server HD key signs + broadcasts.",
  },
];

export interface SigningModeToggleProps {
  value: SigningMode;
  /** persist server-side via PUT /settings/signing-mode; resolve on success */
  onChange: (mode: SigningMode) => Promise<void> | void;
  disabled?: boolean;
}

/**
 * Mutually-exclusive signing-mode radio (impl §5.0). Helper text per spec:
 * "Browser wallet: you pay PLASMA gas. Server key: the clinic's wallet pays."
 * Persisted server-side so the choice follows the issuer.
 */
export function SigningModeToggle({ value, onChange, disabled }: SigningModeToggleProps) {
  const [pending, setPending] = useState<SigningMode | null>(null);

  async function select(mode: SigningMode) {
    if (mode === value || pending) return;
    setPending(mode);
    try {
      await onChange(mode);
    } finally {
      setPending(null);
    }
  }

  return (
    <div role="radiogroup" aria-label="Signing mode" className="grid gap-3 sm:grid-cols-2">
      {OPTIONS.map((opt) => {
        const selected = value === opt.value;
        const isPending = pending === opt.value;
        const Icon = opt.icon;
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={disabled || pending !== null}
            onClick={() => void select(opt.value)}
            className={cn(
              "flex flex-col gap-2 rounded-lg border p-4 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60",
              selected
                ? "border-primary bg-primary/5 ring-1 ring-primary"
                : "border-border bg-surface hover:bg-surface-muted",
            )}
          >
            <div className="flex items-center justify-between">
              <span className="flex items-center gap-2 font-medium text-onSurface">
                <Icon className="h-4 w-4 text-primary" />
                {opt.title}
              </span>
              {isPending ? (
                <Spinner className="h-4 w-4 text-muted" />
              ) : (
                <span
                  className={cn(
                    "flex h-4 w-4 items-center justify-center rounded-full border",
                    selected ? "border-primary" : "border-border",
                  )}
                >
                  {selected && <span className="h-2 w-2 rounded-full bg-primary" />}
                </span>
              )}
            </div>
            <span className="text-sm text-muted">{opt.blurb}</span>
          </button>
        );
      })}
    </div>
  );
}
