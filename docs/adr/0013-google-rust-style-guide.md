# ADR-0013: Follow the Google Rust Style Guide

Status: accepted

## Context
Build time on CI and for local development should stay low as the workspace
grows.

## Decision
Follow the Google Rust Style Guide, with exceptions listed in
`CONTRIBUTING.md`: Chromium- and Fuchsia-only rules do not apply (async with
tokio is used), `Arc` is allowed for shared async state, and `dyn Trait` is
forbidden unless generics cannot express the code.

## Consequences
- Many small crates rather than a few large ones.
- Proc macros are avoided beyond standard derives.
- Use cases are generic over their ports. Each binary wires one adapter per
  port, so each use case is compiled once per binary and monomorphization
  stays cheap.
