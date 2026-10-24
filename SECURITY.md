# Security Policy

## Supported versions

Security fixes ship in the latest release published on crates.io as `ownserp`, on npm as `@devops-infinity/ownserp`, and on the GitHub releases page. Upgrade to that release before reporting an issue that may already be fixed. Pre-release versions receive no separate support.

## Report a vulnerability

Use GitHub private vulnerability reporting: open the repository's Security tab and choose "Report a vulnerability". That channel is private between you and the maintainer.

If private reporting is unavailable, send the report by email to sazzad@devops.bd with "ownserp security" in the subject.

Do not report a vulnerability in a public issue, pull request, or discussion.

## What a report contains

- The affected version, from `ownserp --version`.
- The operating system and how the server runs: stdio, Streamable HTTP, container, or a bundle.
- The steps that reproduce the problem, with the arguments and the configuration that matter.
- The impact you observed, such as a leaked credential, a refused or bypassed authorization, or a crash.
- Any proof of concept, with secrets removed.

## Response

The report gets a reply on its private thread. No fixed acknowledgement or disclosure timeline is promised. A fix ships in a release, and the release notes state what changed.

## Scope

In scope: the code in this repository. That covers tool argument validation, the HTTP bearer gate, Host and body limits, credential handling, the ledger, the response shaping that treats upstream text as untrusted data, and the release scripts.

Out of scope: the Serper.dev service and its own API, the MCP client and its model, the operating system keychain, and vulnerabilities in a third-party dependency, which belong to that dependency's own security process. An API key that leaked through a channel outside this server is handled at https://serper.dev/api-keys by revoking it there.

## Data the server handles

The server reads `SERPER_API_KEY` from the environment or the OS keychain, keeps it in memory, and masks it in every display. It sends the query arguments to the Serper API over TLS. It writes one row per call to a local ledger with a keyed hash of the arguments and never the query text. It sends no telemetry.

`docs/threat-model.md` states the assets, the trust boundaries, the controls, and the residual risks.
