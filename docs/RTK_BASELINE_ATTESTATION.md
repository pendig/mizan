# RTK Baseline Integration Attestation

## License and Attribution

- RTK upstream license check was completed from the canonical public source:
  `https://github.com/rtk-ai/rtk` (Apache-2.0).
- `mizan-rtk` introduces a modular baseline layer inspired by RTK patterns for
  CLI command output filtering and OpenAI-compatible proxying.
- This repository keeps RTK usage as an attribution-aware integration point and
  does not claim RTK trademark ownership.

## Code Baseline

`crates/mizan-rtk` now exposes:

- `filter` module: prompt/response compression and output-size filtering helpers.
- `proxy` module: request builders and OpenAI-compatible proxy entrypoints.

`crates/mizan-cli` now consumes `mizan-rtk` directly via subcommands:

- `mizan-cli filter <text>`
- `mizan-cli proxy --base-url ... --api-key ... --model ... --message ... [--compact] [--json]`

## Audit Notes

- Baseline behavior is now testable through crate unit tests (`mizan-rtk`) and CLI
  execution paths.
