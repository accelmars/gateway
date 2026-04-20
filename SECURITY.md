# Security Policy

## Supported Versions

During the alpha period, only the latest published version receives security fixes.

| Version | Supported |
|---------|----------|
| latest  | Yes      |
| older   | No       |

## Reporting a Vulnerability

Do **not** open a public GitHub issue for security vulnerabilities.

Send a report to: **security@accelmars.com**

**Response SLA:** We acknowledge all reports within 48 hours.

Please include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact assessment
- Affected versions

We will coordinate a fix and disclosure timeline with you. If a CVE is warranted, we will request one. We credit all reporters in the release notes unless anonymity is requested.

## Scope

This policy covers:
- `accelmars-gateway` binary and library
- `accelmars-gateway-core` library
- The gateway HTTP API and routing logic

**Out of scope:**
- Vulnerabilities in third-party provider APIs (Gemini, DeepSeek, Anthropic, etc.)
- Issues arising from misconfigured API keys or insecure deployment environments
- Vulnerabilities in dependencies — report these upstream and to RustSec (https://rustsec.org)
