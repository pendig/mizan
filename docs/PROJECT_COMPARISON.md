# Mizan vs Related Projects (snapshot: 2026-06-17)

This document maps Mizan against projects you mentioned and clarifies where Mizan is similar and where it is intentionally different.

## Why this exists

Mizan is currently a backend/API-first control-plane with minimal UI.
The goal is to avoid a large rewrite by selecting the right baseline and keeping the project scope realistic.

## Project-by-project comparison

### 1) jonradoff/lastsaas

LastSaaS is a full SaaS boilerplate stack.
Its focus is rapid delivery of a generic business platform with multi-tenant auth,
RBAC, Stripe, webhooks, and analytics.

- Similar: admin workflows and product scaffolding
- Different: Mizan is a focused AI gateway, not a full generic SaaS starter
- Risk of using as base: introduces a lot of product assumptions unrelated to AI gateway control and credits

### 2) BerriAI/litellm

LiteLLM is a mature AI gateway with many provider integrations and strong enterprise-adjacent capabilities.
It is broad and has an admin surface for centralized proxying.

- Similar: OpenAI-compatible proxy surface, cost/limit logic, multi-provider routing, admin-oriented features
- Different: Python stack, larger operational footprint, and a recent supply-chain compromise history in 1.82.7/1.82.8
- Reasoning: Mizan borrowed the contract shape but keeps Rust-first scope and simpler first-mile operations

### 3) LiteLLM-Labs/litellm-rust

litellm-rust is a small Rust gateway specifically for coding-agent style flows.

- Similar: Rust compatibility and OpenAI-like route goals
- Different: currently much smaller scope than Mizan today, and currently not a full product-level control-plane by itself
- Use-case fit: good reference for Rust gateway patterns, not a full replacement baseline

### 4) maximhq/bifrost

Bifrost is an enterprise-grade AI gateway with strong emphasis on performance,
failover, clustering, and plugin architecture.

- Similar: gateway-first architecture and routing/governance controls
- Different: much broader enterprise gateway surface and heavier operational feature set for this project stage
- Use-case fit: strong for high-scale gateway teams, less aligned with current minimal MVP footprint

### 5) api7/aisix

AISIX positions as a Rust AI gateway with routing, caching, observability, and policy features.

- Similar: OpenAI-compatible direction and Rust positioning
- Different: currently smaller community signals and different current production readiness profile compared to mizan target
- Use-case fit: useful for ideas on provider lifecycle and policy design, not a drop-in replacement for current mizan MVP

## What is uniquely practical in Mizan now

Mizan sits between a tiny internal gateway and a full SaaS control-plane.

1. Internal credit accounting with ledger-first model
2. Lightweight virtual key lifecycle and role split
3. Explicit path from API release -> thin UI
4. RTK baseline integration for CLI workflows
5. Rust-first architecture with SQLite + Redis defaults

## Recommendation

For current MVP speed, continue with Mizan as-is:

- keep backend/API correctness first,
- use a small UI as a dedicated next phase,
- avoid rewriting into a full-featured external baseline that dilutes the objective.
