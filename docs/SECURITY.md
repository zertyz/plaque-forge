# Security implementation and reporting

Current controls and recorded exceptions. Normative requirements: [Security](../management/SECURITY.md).

## Reporting

Follow [S-004](../management/SECURITY.md#s-004-security-reporting). Report privately to the maintainer through GitHub's private vulnerability reporting when enabled, or an established private contact. Do not publish sensitive details merely because a private reporting route is unavailable.

## Automated dependency gate

CI runs the RustSec advisory database against the committed `Cargo.lock`. Vulnerabilities, yanked dependencies, unsoundness notices, and new maintenance warnings fail the security job. Run the same gate locally with:

```bash
cargo install cargo-audit --version 0.22.2 --locked
./scripts/audit_dependencies.sh
```

The scan refreshes the advisory database over the network. The local command specifies `cargo-audit 0.22.2`, but the current CI install omits `--version`. [WI-030](../management/SECURITY.backlog.md#wi-030-resolve-audit-policy-and-exception-drift) tracks that mismatch; the new fail-with-remediation requirement is not yet fully enforced.

## Tracked upstream exception

The existing record identifies `RUSTSEC-2026-0192` as an unmaintained notice for `ttf-parser 0.25.1`, received through `cosmic-text 0.19 -> fontdb 0.23`, with no patched release listed at review. The audit helper ignores that exact notice. This is the recorded exception, not a fresh advisory assessment.

The recorded review date is 2026-08-12; the next review is due on each `cosmic-text` update and no later than 2026-11-12. Its stated removal condition is an upstream migration away from the parser or a pixel/layout-equivalent typography replacement. Reassessment and any amendment follow [S-003](../management/SECURITY.md#s-003-dependency-checks-and-exceptions); this documentation consolidation does not renew or remove the exception.

## Artifact and input boundaries

- Generated manifests may contain portable repository-relative identifiers, hashes, dimensions, and tool versions, but never absolute workstation paths or model credentials.
- Analysis caches are untrusted until schema, provenance, dimensions, masks, prompts, and portable paths pass validation.
- External tools receive argument arrays rather than shell command strings.
- Intermediate frames, requests, and model caches live below `/tmp/plaque-forge*`; successful work is removed automatically and retained failure evidence is bounded.
