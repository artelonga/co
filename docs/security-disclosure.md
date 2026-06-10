# Security Disclosure Process

This document describes the manual disclosure flow for **Critical** security findings
detected by the CO-388 security audit pipeline.

> **Automation status**: This flow is manual for v3.2. Auto-CVE submission and
> coordinated multi-party disclosure are deferred to v3.3+.

## Severity Definitions

| Severity | Definition | Response SLA |
|----------|-----------|--------------|
| **Critical** | Remote code execution, auth bypass, data exfiltration | Patch within 24h |
| **High** | Privilege escalation, significant data exposure | Patch within 72h |
| **Medium** | Limited impact, requires user interaction | Next sprint PBI |
| **Low / Info** | Best-practice improvements | Advisory only |

## Steps for Critical Findings

### 1. Finding detected — immediate response

When the security audit pipeline detects a Critical finding:

1. The CI job blocks the PR (`pr-route.yml` step 11 exits non-zero).
2. A `security.release_blocked` event is published to the EDA bus.
3. The finding is written to `security_findings` table (ID = ULID).
4. A PBI is auto-created at `work/co/security/SEC-<id>.md`.
5. Yuri receives a 🚨 alert in `/agora`.

**Do not merge until the finding is resolved.**

### 2. Verify the finding

```bash
# Check the finding in the admin API
curl -H 'Authorization: Bearer <token>' \
  https://co-artelonga.fly.dev/api/v1/gestao/security/findings/<id>
```

Determine: is this a genuine vulnerability or a false positive?

- **False positive** → resolve immediately:
  ```bash
  curl -X PATCH .../findings/<id> \
    -d '{"resolution_kind":"false-positive"}'
  ```
  Document why in a PR comment.

- **Genuine** → proceed to step 3.

### 3. Create a private GitHub Security Advisory (genuine Critical/High)

1. Navigate to: `github.com/artelonga/co` → Security → Advisories → New
2. Fill in:
   - **Title**: brief description (do not include exploit details)
   - **Severity**: use CVSS v3.1 calculator
   - **CWE**: from the finding's `cwe` field
   - **Affected versions**: current stable release
   - **Patch**: describe the fix (no exploit PoC)
3. Save as **Draft** (not published yet).
4. Note the GHSA-XXXX-XXXX-XXXX identifier for reference.

### 4. Apply the fix

1. Create a hotfix branch from `main`:
   ```bash
   git checkout -b fix/CO-N-security-<cve-stub>
   ```

2. Write a failing test that demonstrates the vulnerability (red).

3. Apply the minimal fix (green).

4. Run the full test suite:
   ```bash
   cargo test && cargo clippy -- -D warnings && cargo fmt
   ```

5. Open a PR with title: `fix(security): CO-N — resolve Critical <category> finding`

6. After the PR passes CI (including step 11), merge it.

7. Resolve the finding via the API:
   ```bash
   curl -X PATCH .../findings/<id> \
     -d '{"resolution_kind":"patched","resolution_pr":<pr-number>}'
   ```

### 5. Deploy the patch

1. Deploy to production via the normal release process:
   ```bash
   scripts/release-commit.sh <version> security-patch
   ```

2. Verify the finding does not recur on the patched deployment.

### 6. Coordinate disclosure (CVE-eligible findings)

If the finding affects CO users or third-party software:

1. Check if the affected library already has a CVE.
2. If not, request a CVE via [MITRE CVE Program](https://cveform.mitre.org/) or
   the GitHub Advisory Database.
3. Wait for CVE assignment before public disclosure.

### 7. Public disclosure (≥7 days after patch ships)

Standard responsible-disclosure timeline:
- Patch ships → start 7-day countdown
- After 7 days → publish the GitHub Security Advisory (change from Draft to Published)
- Post a brief note in CHANGELOG.md under the patched version

## Emergency Override

If a Critical/High finding must be bypassed for an emergency hotfix:

```bash
# In GitHub Actions — workflow_dispatch
ignore_security_findings: true
```

**This is logged and alerted.** A `security.override_activated` event is published
to the EDA bus and the audit trail captures the timestamp. The override MUST be
followed by a fix PR within 24h (Critical) or 72h (High).

Never use the override to permanently avoid fixing a vulnerability.

## Contact

Security findings or responsible-disclosure reports from external researchers:

- Email: yuri@artelonga.com.br (subject: `[SECURITY]`)
- GitHub: open a private security advisory at `github.com/artelonga/co/security/advisories`

Response SLA: 48h acknowledgment, 7 days for triage, patching per the table above.
