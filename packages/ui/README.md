# @dogtag/ui

Shared portal UI library for the DogTag ecosystem — the reference implementation of all
shared portal patterns (vet, groomer, admin reuse this). React 18 + TypeScript + Tailwind +
shadcn-style primitives, with wallet-connect and the on-chain verify flow.

Consumed **as workspace source** (no prebuild step): apps import from `@dogtag/ui` and Vite
transpiles the `.ts`/`.tsx` in place. `pnpm --filter @dogtag/ui build` runs `tsc --noEmit`
(strict typecheck) as the gate.

## What's in here

- **Theming** (impl §5.0): semantic design tokens as CSS variables with **light + dark**
  palettes (`src/tokens.css`), a Tailwind preset mapping them to utilities
  (`src/tailwind-preset.ts`), `ThemeProvider` / `useTheme` (localStorage-persisted),
  `ThemeToggle`. Components reference **semantic tokens only** (`bg-surface`, `text-onPrimary`…).
- **Primitives** (Radix-based where useful): `Button`, `Card` (+ Header/Title/Description/
  Content/Footer), `Input`, `Label`, `Select`, `Dialog`, `Badge`, `Table`, `Tabs`,
  `Toast` (`ToastProvider` + `useToast`), `Spinner`, `QrCode` (qrcode.react), and `AppShell`
  (dark sidebar + light content, matching the groomer reference).
- **Wallet** (wagmi v2 + viem 2 + Reown AppKit): `roax` chain (`defineChain`, id 135 / 0x87,
  native PLASMA), `WalletProvider` (WagmiProvider + react-query + AppKit init), `WalletButton`,
  and `useRoaxChain()` — `wallet_switchEthereumChain` → on 4902 `wallet_addEthereumChain`.
- **Domain components**: `SigningModeToggle` (wallet ⟷ server key), `StatusPanel`
  (wallet: address + ROAX + whitelist badges / backend: genesis + signer + PLASMA balance),
  `VerifyFlow` (purpose + Normal/ZK toggle → session QR → poll → on-chain status).
- **API client** (`createApiClient`): typed factory covering the vet backend + central
  issuer-application endpoints; wire types in `src/api/types.ts` mirror the Rust serde contracts.
- **Schema** (`src/schema/recordTypes.ts`): schema-driven issue-form descriptors per impl §1.6
  (RabiesVaccinationCertificate, DOG_PROFILE), with client-side validators.

## Usage notes

- `createWalletConfig` reads the Reown projectId from `import.meta.env.VITE_REOWN_PROJECT_ID`
  with a placeholder default. WalletConnect transport requires a real Reown Cloud projectId.
- Apps add `@tailwind` directives in their own entry CSS and extend the shared preset; import
  `@dogtag/ui/tokens.css` for the design tokens (or `@dogtag/ui/styles.css` for the all-in-one).
