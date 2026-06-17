# GitHub Open Source Readiness Checklist

Target date: 2026-06-17

Mizan should feel familiar to contributors who expect modern open-source repository hygiene.

## Required (already in-repo)
- [x] LICENSE (Apache-2.0)
- [x] CONTRIBUTING.md
- [x] SECURITY.md
- [x] CODE_OF_CONDUCT.md
- [x] README with install/usage/intent/status
- [x] CHANGELOG.md
- [x] CI workflow (`.github/workflows/ci.yml`)
- [x] Dependabot configuration
- [x] Issue templates and PR template
- [ ] `CODEOWNERS` (recommended)
- [ ] `FUNDING.yml` (optional, if donation/model available)

## Recommended repository settings
- [ ] Restrict `main` to PR merges only
- [ ] Require CI check before merge
- [ ] Enable code-owner review for core files (if CODEOWNERS is used)
- [ ] Enable Dependabot security updates
- [ ] Enable secret scanning alerts
- [ ] Enable branch protection for force-push prevention
- [ ] Enable issue transfer to Discussions workflow if support is needed

## Release hygiene checklist
- [ ] Keep release notes aligned with release tags
- [ ] Publish signed release artifacts when available
- [ ] Link release note section in README/CHANGELOG
- [ ] Ensure reproducible docs and smoke scripts are referenced in releases

## Contributor workflow expectations
- [ ] Keep PR scope narrow
- [ ] Link issue/feature discussion to PR
- [ ] Run at least `cargo fmt --all`, `cargo test --workspace` before merge
- [ ] Update docs for any API contract or behavior changes

## Public communication
- [ ] Add clear support channel in GitHub profile/README
- [ ] Keep roadmap and known limitations visible
- [ ] Keep comparison and architecture docs updated with dated references

