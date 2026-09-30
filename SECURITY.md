# Security policy

phpm downloads and writes code into your project, so security reports are
taken seriously.

## Reporting

Use GitHub's private vulnerability reporting:
<https://github.com/speedsharmaai/phpm/security/advisories/new>.
Please do not open a public issue.

You will get an acknowledgement within 72 hours and a plan within 7 days.

## Supported versions

phpm is pre-release. Only the latest `main` is supported until 1.0.

## Scope

In scope: anything that makes phpm install different code than Composer
would, bypass Composer's malware filter or advisories, leak credentials from
`auth.json`, or write outside the project and cache directories (path
traversal in archives, symlinks, bin proxies).
