# DogTag monorepo — root task runner (just is unavailable; GNU Make 3.81)
.DEFAULT_GOAL := help
.PHONY: help dev build test parity sdk-ts sdk-rs contracts deploy-contracts clean up-admin up-vet up-groomer up-government up-indexer test-consent-parity vendor-mobile-artifacts check-cutover-consumers

help: ## list targets
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

## ---- build / test ----
build: sdk-ts sdk-rs contracts ## build everything buildable

test: check-cutover-consumers parity test-ts test-rs test-contracts ## run all test suites

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

contracts: ## compile Foundry contracts (incl. the S-12 rehearsal suite - no network needed)
	cd contracts && forge build
	# The rehearsal suite is outside the default profile's source dirs, so nothing else compiles it.
	# Compile-only: this needs NO endpoint and runs no fork test, but it means API drift in
	# ProviderRegistry/DogTagIssuerFactoryV2/CloneProvenanceRouter breaks the build instead of
	# surfacing at the moment someone actually needs the rehearsal.
	cd contracts && FOUNDRY_PROFILE=rehearsal forge build

test-contracts: ## Foundry tests
	cd contracts && forge test -vvv

rehearse-cutover: ## S-12 registry cutover rehearsal on a ROAX FORK - deploys nothing live (NOT in `test`)
	scripts/rehearse-cutover.sh
	scripts/render-cutover-txlist.py

rehearse-cutover-mutations: ## prove every cutover assertion can fail (NOT in `test`)
	ROAX_FORK_RPC=$${ROAX_FORK_RPC:-https://devrpc.roax.net} scripts/rehearsal-mutations.sh

# Deliberately part of `test`, unlike test-consent-parity and rehearse-cutover*, which are excluded
# because they are slow or need an endpoint. This is a few git greps plus python and needs neither.
# An unrun gate has exactly the property it objects to in a hand-maintained list: a newly-added
# consumer stays invisible until someone remembers to look. It runs FIRST because it is the cheapest.
check-cutover-consumers: ## S-13 client-repoint inventory gate (hermetic, no network) - runs in `test`
	scripts/check-cutover-consumers.sh

deploy-contracts: ## deploy to ROAX (requires liveness precheck — see script/Deploy.s.sol)
	cd contracts && forge script script/Deploy.s.sol --rpc-url $${ROAX_RPC:-https://devrpc.roax.net} --broadcast

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
