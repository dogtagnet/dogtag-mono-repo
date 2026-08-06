# DogTag monorepo — root task runner (just is unavailable; GNU Make 3.81)
.DEFAULT_GOAL := help
.PHONY: help dev build test check-addresses test-address-gate test-e2e-lib e2e e2e-web e2e-ios e2e-android parity sdk-ts sdk-rs contracts deploy-contracts clean up-admin up-vet up-groomer up-government up-indexer test-consent-parity vendor-mobile-artifacts verify-provider-selfservice-mutations verify-content-mirror-mutations verify-deployment-record-mutations

help: ## list targets
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

## ---- build / test ----
build: sdk-ts sdk-rs contracts ## build everything buildable

test: check-addresses test-address-gate test-e2e-lib parity test-ts test-rs test-contracts ## run all test suites

check-addresses: ## assert no consumer hardcodes a deployed address (hermetic) - runs in `test`
	bash scripts/check-no-hardcoded-addresses.sh

test-address-gate: ## prove the address gate actually guards (3.6s, hermetic temp repos) - runs in `test`
	bash scripts/test-address-gate.sh

test-e2e-lib: ## pin the e2e runners' watchdog + could-not-run classifier (hermetic) - runs in `test`
	bash scripts/test-e2e-lib.sh

## ---- end-to-end (NOT in `test`: they serve portals / need a device) ----
# Each SCRIPT exits 0 pass, 1 fail, and 78 "DID NOT RUN" when a prerequisite is genuinely absent. A
# skipped suite is treated as a FAILURE, never as a pass - see the header of scripts/lib/e2e.sh.
#
# CALL THE SCRIPT, NOT `make`, WHEN THE EXIT CODE MATTERS. GNU make always exits 2 when a recipe
# fails, so it collapses 1 and 78 into one number and the whole three-outcome contract disappears at
# the entry point that documents it. The banner still prints, so `make` is fine to read; anything
# that BRANCHES on the outcome must run `bash scripts/e2e-<surface>.sh` directly.
e2e: e2e-web e2e-ios e2e-android ## every e2e surface (web + both mobile platforms)

e2e-web: ## browser e2e: starts the portals + a hermetic government backend, runs Playwright, tears down
	bash scripts/e2e-web.sh $(if $(ONLY),--only $(ONLY),)

e2e-ios: ## iOS e2e: builds the app, boots a simulator, runs the Maestro flows
	bash scripts/e2e-ios.sh

e2e-android: ## Android e2e: builds the apk, installs on a connected arm64 device/emulator, runs Maestro
	bash scripts/e2e-android.sh

parity: ## NORMATIVE Poseidon 4-language anchor gate (t=2/3/6/7) — BLOCKS downstream
	cd circuits && pnpm run parity

sdk-ts: ## build the TS standard SDK
	pnpm --filter @dogtag/standard build

sdk-rs: ## build the Rust standard crate
	cargo build -p dogtag-standard-rs

test-ts: ## TS SDK + shared-UI tests (incl. shared testvectors.json)
	pnpm --filter @dogtag/standard test
	# `@dogtag/ui` carries the decision layer behind every portal — verdict folds, action-availability
	# reasons, the provider engine, and now the issuance-list page. It IS in `.no-mistakes.yaml`'s
	# `commands.test`, but that gate is PAUSED fleet-wide, so in practice its ~877 tests only ran when
	# somebody remembered to — while `make test`, the thing people actually run, reported 779 and said
	# nothing about them. Named explicitly rather than broadened to `pnpm -r test`, which would sweep
	# in Playwright specs that drive live portals and anchor real records on chain.
	pnpm --filter @dogtag/ui test
	# `@dogtag/admin-web` is the SAME gap one package over: it carries the registrar surface's own
	# mounted suites - the demo fill that must not supply a controller key, the chain-ref rendering -
	# and it was reachable only through the paused no-mistakes gate, so `make test` said nothing about
	# it either. Safe to name here: it has no `e2e/` directory at all, and its vitest config scopes
	# `include` to `test/**`, so this cannot sweep in a live-portal driver.
	pnpm --filter @dogtag/admin-web test

test-rs: ## Rust SDK tests (incl. shared testvectors.json)
	cargo test -p dogtag-standard-rs

test-consent-parity: ## consent prove<->VK parity - LOUD gate, fails if artifacts absent (NOT in `test`)
	scripts/test-consent-parity.sh

vendor-mobile-artifacts: ## copy the consent zkey+graph from circuits/build into both app bundles
	scripts/vendor-mobile-artifacts.sh

gen-mobile-config: ## write both apps/*/roax.json address bundles from the deploy ledger
	scripts/gen-mobile-roax-config.sh

contracts: ## compile Foundry contracts
	cd contracts && forge build

test-contracts: ## Foundry tests
	cd contracts && forge test -vvv

verify-provider-selfservice-mutations: ## S-15 mutation gate for the provider engine (slow; not in `test`)
	bash scripts/verify-provider-selfservice-mutations.sh

verify-signer-roster-mutations: ## mutation gate for the issuance-list surface (slow; not in `test`)
	bash scripts/verify-signer-roster-mutations.sh

verify-address-config-mutations: ## mutation gate for addresses-as-configuration (slow; not in `test`)
	bash scripts/verify-address-config-mutations.sh

verify-content-mirror-mutations: ## S-17 mutation gate for the content mirror (slow; not in `test`)
	bash scripts/verify-content-mirror-mutations.sh

verify-deployment-record-mutations: ## mutation gate for the deploy record + the stranded states (slow; not in `test`)
	bash scripts/verify-deployment-record-mutations.sh

# ADMIN must be the broadcasting key: the registrar wiring inside the script is onlyOwner on a core it
# has just handed to ADMIN. CUSTODIAN must be a neutral sink - no code, no role, never signs.
deploy-contracts: ## deploy the whole set to ROAX (needs ADMIN + CUSTODIAN; see script/Deploy.s.sol)
	cd contracts && forge script script/Deploy.s.sol:Deploy \
		--rpc-url $${ROAX_RPC:-https://devrpc.roax.net} --broadcast --legacy --slow

## ---- stacks ----
up-admin:   ## docker compose up the central/admin stack (39741/39742)
	cd stacks/admin && docker compose up -d
up-vet:     ## docker compose up the vet stack (41873/41874)
	cd stacks/vet && docker compose up -d
up-groomer: ## docker compose up the groomer stack (43617/43618)
	cd stacks/groomer && docker compose up -d
up-government: ## docker compose up the government stack (44831/44832)
	cd stacks/government && docker compose up -d
up-indexer: ## docker compose up the oversight indexer stack (46001, backend-only)
	cd stacks/indexer && docker compose up -d

clean: ## remove build artifacts
	rm -rf node_modules packages/*/dist packages/*/node_modules target contracts/out contracts/cache
