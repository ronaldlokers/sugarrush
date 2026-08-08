# Security policy

## Reporting a vulnerability

Please report security issues privately through
[GitHub Security Advisories](https://github.com/ronaldlokers/sugarrush/security/advisories/new)
rather than a public issue. If that isn't an option, email
<ronald@lokers.email>.

Include what you did, what happened, and the version (`sugarrush about`).
Expect an acknowledgement within a few days. This is a personal project, not a
company with an on-call rotation — a fix lands as soon as I can get to it, and
you'll be credited in the release notes unless you'd rather not be.

## Supported versions

The latest release only. Fixes ship as a new CalVer release rather than being
backported.

## What sugarrush touches

Worth knowing when judging whether something is a vulnerability:

- **Your Nightscout token is stored in plaintext** in `config.toml`, by design
  — it's a read-only token, and the file is created `0600`. The app warns in
  the footer if the file becomes group- or world-readable. Token handling
  (permissions, atomic writes, masked entry, no rendering back to screen) *is*
  in scope; the plaintext-at-rest design decision is documented, not a bug.
- **The token is sent as a `?token=` query parameter**, which is what the
  Nightscout API accepts. Over `https` that's inside TLS; over plain `http` it
  is not, which the app warns about.
- **It only reads.** sugarrush never writes to your Nightscout site, and the
  token it asks for cannot.
- **It talks to your site and, optionally, one webhook** (`push_url`) that you
  configure. There is no telemetry and no other network egress.

## Not a medical device

sugarrush displays CGM data; it is not a medical device and must not be used
for treatment decisions. A bug that causes a *wrong or missing alarm* is still
a serious bug and worth reporting as one — file it as a regular issue unless
it's exploitable by someone else.
