# START HERE — DogTag build orchestration prompt

> Paste the block below into a fresh Claude Code session opened in `dogtag-mono-repo/`,
> or just tell the session: "follow docs/START_HERE.md".

---

You are building the DogTag pet-credentialing ecosystem in the current repo
(/Users/zhenhaowu/code/dogtag-mono-repo). A complete, 12-times-audited specification
already exists in docs/. Your job: IMPLEMENT it, phase by phase, orchestrating
sub-agents and tracking everything in a todo list.

═══ SOURCE OF TRUTH (read these FIRST, in full, before any code) ═══
• docs/architecture.md   — system + contract architecture, the Poseidon data standard,
                           verification pipeline, privacy model.
• docs/implementation.md — per-function pseudocode, full contract bodies, every endpoint,
                           Docker, deploy, and the testing strategy.
• docs/BUILD_PROMPT.md    — the phased plan (Phases 1→8 incl. 2.5), the 9 non-negotiable
                           principles, and each phase's acceptance criteria.
• docs/research/          — 13 research briefs + 12 audit reports + CHANGESPEC v2/v3/v4 (the "why").

NORMATIVE PRECEDENCE (critical): section numbering is non-sequential because audit-remediation
blocks were appended. Within architecture.md §13 and implementation.md §11, the
HIGHEST-NUMBERED sub-section wins (§13.9 / §11.10 = v4.1 Poseidon unification are the latest);
those §13/§11 blocks override earlier sections. CHANGESPEC-v4 overrides any earlier
hash/dual-root wording. circomlib is the reference-of-record for Poseidon.

═══ STEP 1 — PLAN FIRST (no code yet) ═══
1. Read the four sources above. Skim the audit reports; read the briefs relevant to each phase.
2. Create a MASTER TODO LIST (TodoWrite) with one tracked item per phase (1, 2, 2.5, 3, 4, 5, 6,
   7, 8), plus the blocking gates and each phase's acceptance criteria as sub-items. This list is
   your single source of progress truth — check an item off ONLY when its acceptance criteria pass.
3. Present the plan + todo list, then proceed to Step 2.

═══ STEP 2 — EXECUTE PHASE BY PHASE (spawn sub-agents for everything) ═══
Build in BUILD_PROMPT order. Do NOT start a phase until the prior phase's acceptance criteria are
green. For each phase: decompose into independent units, SPAWN A SUB-AGENT PER UNIT (Agent tool),
partition by package/folder so agents don't collide, use git-worktree isolation for agents that
write files in parallel. Each sub-agent implements faithfully (pseudocode → real code) + writes
tests + reports results. Then integrate, run the phase's acceptance tests yourself, fix gaps,
check off the todo, and COMMIT on a feature branch (never the default branch).

Suggested decomposition:
• Phase 1 (trust core): A=packages/dogtag-standard-ts, B=crates/dogtag-standard-rs,
  C=generate shared testvectors.json FROM circomlib incl. per-arity Poseidon anchors (t=2/3/6/7)
  + leaf/Merkle/nullifier vectors. Reconcile until TS == Rust == circomlib bit-identical.
• Phase 2 (contracts): contracts/ (IssuerRegistry, DogTagIssuer, factory, DogTagSBT, write-once
  rootIssuer index) + Foundry tests incl. ALL audit regression tests.
• Phase 2.5 (ZK): A=circuits/ (circom Poseidon-Merkle + EdDSA-BabyJubjub consent, trusted setup),
  B=VerificationRegistry + ConsentKeyRegistry + Groth16Verifier + Foundry tests (both paths,
  shared-nullifier double-spend, range-checks, relayer binding, subject↔key/ownerOf/purpose).
• Phase 3 (vet backend) + Phase 4 (central backend): one agent per service area.
• Phase 5 (portals): agent per app (packages/ui, vet, groomer, admin).
• Phase 6 (mobile): Android agent, iOS agent, UniFFI-bindings agent.
• Phase 7 (calendar/appointments) + Phase 8 (E2E hardening, re-run the audit lenses on the CODE,
  privacy/erasure gate, dual-signing parity test).

═══ BLOCKING GATES (do NOT bypass) ═══
• Poseidon parity (Phase 1): per-arity (t=2/3/6/7) circomlib-referenced anchors bit-identical
  across circom/TS/Rust/Solidity. Block everything downstream until green.
• Security gates (before any deploy): all audit Criticals/Highs from §13/§11 implemented —
  _disableInitializers, per-recordType isWhitelistedFor, issuedBy originator, write-once
  rootIssuer[R] clone resolution, subject↔key + ownerOf + purpose binding, hardened confirm.
• Pre-deploy prechecks: `cast chain-id --rpc-url https://devrpc.roax.net` returns 135 (it was 502
  at design time — confirm liveness), AND ROAX supports the BN254 pairing precompiles
  (ecAdd/ecMul/ecPairing) needed for Groth16. If either fails: STOP and report.
• Privacy: nothing personal on-chain, ever; dogTagId is never any hash of the microchip.

═══ OPERATING RULES ═══
• Tests as you go; a phase is "done" only when its acceptance criteria pass. Report failures
  honestly with output. • If a spec gap or contradiction appears, STOP and surface it with a
  recommendation — don't guess. • Keep the todo list current; checkpoint per phase so work can
  resume from the todo list + git state.

Start now: read the sources, build the master todo list, present the plan.
