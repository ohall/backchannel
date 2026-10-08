# Public test fixtures, not credentials

These RSA keys were generated solely for the local OAuth validation tests. The
private key is intentionally public and must never be configured in a real
identity provider, used to issue real credentials, or trusted by a deployment.
The JWKS fixture is served only by an ephemeral test server inside `#[cfg(test)]`.
