# DogTag monorepo — root task runner (just is unavailable; GNU Make 3.81)
.DEFAULT_GOAL := help
.PHONY: help dev build test check-addresses parity sdk-ts sdk-rs contracts deploy-contracts clean up-admin up-vet up-groomer up-government up-indexer test-consent-parity vendor-mobile-artifacts verify-provider-selfservice-mutations verify-content-mirror-mutations

help: ## list targets
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

## ---- build / test ----
build: sdk-ts sdk-rs contracts ## build everything buildable

test: check-addresses parity test-ts test-rs test-contracts ## run all test suites

check-addresses: ## assert no consumer hardcodes a deployed address (hermetic) - runs in `test`
	bash scripts/check-no-hardcoded-addresses.sh

parity: ## NORMATIVE Poseidon 4-language anchor gate (t=2/3/6/7) — BLOCKS downstream
	cd circuits && pnpm run parity

sdk-ts: ## build the TS standard SDK
	pnpm --filter @dogtag/standard build

sdk-rs: ## build the Rust standard crate
	cargo build -p dogtag-standard-rs

test-ts: ## TS SDK tests (incl. shared testvectors.json)
	pnpm --filter @dogtag/standard test

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

verify-address-config-mutations: ## mutation gate for addresses-as-configuration (slow; not in `test`)
	bash scripts/verify-address-config-mutations.sh

verify-content-mirror-mutations: ## S-17 mutation gate for the content mirror (slow; not in `test`)
	bash scripts/verify-content-mirror-mutations.sh

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
