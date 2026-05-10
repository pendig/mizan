# Security Policy

Mizan is in early bootstrap. Please treat every credential and provider
connection as sensitive.

## Do Not Commit

- Provider API keys.
- Subscription credentials.
- User API keys.
- Local `CONTEXT.md` agent notes.
- Prompt or response logs that may contain private data.

## Core Security Expectations

- Hash virtual API keys.
- Hash user passwords.
- Encrypt provider secrets at rest.
- Store `MIZAN_PROVIDER_SECRET_KEY` securely and rotate it on compromise.
- Never return provider credentials from APIs.
- Disable raw prompt/response logging by default.
- Audit admin changes to providers, model routes, pricing, and credits.
- Use idempotency keys for external credit grants.

## Reporting

Until a formal security contact exists, open a private maintainer channel or
draft advisory rather than filing a public issue with exploit details.
