# Security policy

## Supported version

Security fixes target the current `main` branch and the newest Plaque Forge release. Please report a suspected vulnerability privately to the project maintainers before publishing exploit details. Do not include private media, model credentials, or workstation paths in a public issue.

## Automated dependency gate

CI runs the RustSec advisory database against the committed `Cargo.lock`. Vulnerabilities, yanked dependencies, unsoundness notices, and new maintenance warnings fail the security job. Run the same gate locally with:

```bash
cargo install cargo-audit --version 0.22.2 --locked
./scripts/audit_dependencies.sh
```

The scan requires network access to refresh the advisory database. Pinning the scanner version makes CI behavior reviewable; the advisory database itself intentionally remains current.

## Tracked upstream exception

`RUSTSEC-2026-0192` marks `ttf-parser 0.25.1` as unmaintained. This is not a reported vulnerability, and the advisory lists no patched release. Plaque Forge receives it transitively through `cosmic-text 0.19 -> fontdb 0.23`. The audit command ignores only that exact notice while continuing to deny every other warning.

The exception was reviewed on 2026-08-12. Remove it as soon as `cosmic-text` adopts a `fontdb` release that no longer depends on the unmaintained parser, or replace the typography stack after pixel- and layout-equivalence tests. Review it again on every `cosmic-text` update and no later than 2026-11-12.

## Artifact and input boundaries

- Generated manifests may contain portable repository-relative identifiers, hashes, dimensions, and tool versions, but never absolute workstation paths or model credentials.
- Analysis caches are untrusted until schema, provenance, dimensions, masks, prompts, and portable paths pass validation.
- External tools receive argument arrays rather than shell command strings.
- Intermediate frames, requests, and model caches live below `/tmp/plaque-forge*`; successful work is removed automatically and retained failure evidence is bounded.
