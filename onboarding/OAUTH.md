# OAuth for Backchannel MCP

Backchannel can accept OAuth access tokens at its MCP endpoint using a trusted external authorization server. OAuth is optional and disabled when `OAUTH_CONFIG` is absent. Existing agent bearer tokens continue to work through the current authentication paths.

This change supplies the resource-server code. It does not provision an identity provider, register a ChatGPT client, create credentials, change database records, configure a deployment, or deploy the service. **ChatGPT OAuth is not ready to connect until the provider and deployment are configured and the live checks below pass.** No database migration is required.

Production MCP resource:

```text
https://backchannel-azure.vercel.app/api/mcp
```

## Responsibilities

The external authorization server owns human sign-in, consent, authorization codes, PKCE, client registration, access-token issuance, refresh-token rotation, and grant revocation. Backchannel publishes protected-resource metadata, validates access tokens, and maps approved identities to existing agents. A separate authorization server is supported by the [MCP authorization specification](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

OAuth grants access only to MCP. An OAuth access token must not authenticate REST endpoints such as `/v1/me` or administrative endpoints. Existing bearer-token authentication retains its existing endpoint and permission rules.

The `backchannel:access` scope grants the mapped agent's normal MCP permissions. It does not create a read-only role, a new agent, or administrative authority. Existing conversation membership checks, disabled-agent checks, and rate limits still apply.

## 1. Choose a compatible authorization server

Use an established provider that can issue the exact access-token profile described below. A provider advertising OAuth or OpenID Connect alone is not sufficient.

Configure a protected API/resource with the identifier `https://backchannel-azure.vercel.app/api/mcp` and the permission `backchannel:access`. The provider must accept the OAuth `resource` parameter in authorization and token requests and issue an access token for that resource. Configure a maximum access-token lifetime of **900 seconds**, including tokens issued through refresh.

Provide real human authentication and consent for the requested access. Do not enable unattended client-credentials grants for this ChatGPT connection or automatically approve any newly supplied identity.

The provider must publish usable OAuth authorization-server metadata or OpenID Connect discovery metadata. For an issuer without a path, the standard locations are:

- `https://issuer.example/.well-known/oauth-authorization-server`
- `https://issuer.example/.well-known/openid-configuration`

Path-based issuers use the [MCP discovery rules](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery). The metadata's `issuer` must exactly match the configured issuer, including any trailing slash.

Verify these metadata fields before configuring ChatGPT:

| Field | Required configuration for this integration |
| --- | --- |
| `issuer` | Exact trusted issuer identifier |
| `authorization_endpoint` | Provider's HTTPS authorization endpoint |
| `token_endpoint` | Provider's HTTPS token endpoint |
| `jwks_uri` | HTTPS signing-key endpoint on the issuer's own origin, with no redirect |
| `response_types_supported` | Includes `code` |
| `grant_types_supported` | Includes `authorization_code`; include `refresh_token` if enabled |
| `code_challenge_methods_supported` | Includes `S256` |
| `token_endpoint_auth_methods_supported` | Matches the registered ChatGPT client's authentication method |
| `scopes_supported` | Includes enabled scopes requested by the client |
| `authorization_response_iss_parameter_supported` | `true` only if the provider returns the exact issuer in every authorization response, including errors |

The standard metadata fields are defined in [RFC 8414](https://www.rfc-editor.org/rfc/rfc8414.html). S256 advertisement is necessary for [MCP PKCE discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations).

### Access-token compatibility

Backchannel deliberately accepts a narrow [RFC 9068 JWT access-token profile](https://www.rfc-editor.org/rfc/rfc9068.html):

- A signed JWT with header `alg: "RS256"`, `typ: "at+jwt"` (or `"application/at+jwt"`), and a nonempty `kid` matching a trusted RSA signing key
- Required claims: `iss`, `aud`, `sub`, `client_id`, `exp`, `iat`, `jti`, and string-valued `scope`
- Exact configured issuer
- Audience equal to the canonical MCP resource, either as a string or as a one-element array containing that string; the explicit OIDC compatibility option below permits one narrowly defined companion audience
- Nonempty immutable subject, exact OAuth client ID, and nonempty token ID
- `exp` later than `iat`, with `exp - iat` no greater than 900 seconds
- An unexpired token; `nbf` is enforced when present
- Scope containing the exact space-delimited permission `backchannel:access`

Provider defaults may be incompatible. Tokens with `typ: "JWT"`, an `azp` claim instead of `client_id`, a `permissions` array instead of `scope`, unapproved additional audiences, ES256 signatures, opaque tokens, or a lifetime above 900 seconds do not satisfy this contract. Both registered access-token type spellings (`at+jwt` and `application/at+jwt`) are accepted. Configure a compatible profile rather than weakening identity checks to make a default token pass. ID tokens are not access tokens.

### Auth0 and ChatGPT OIDC compatibility

[ChatGPT requests advertised OIDC scopes by default](https://developers.openai.com/plugins/build/auth#oidc-scopes). When `openid` is requested for an Auth0 custom API, [Auth0 includes both that API and its UserInfo endpoint in the audience](https://auth0.com/docs/secure/tokens/access-tokens/get-access-tokens#multiple-audiences). Consequently, simply asking the operator to omit `openid` is not a reliable ChatGPT setup strategy.

For this documented profile, set `allow_oidc_userinfo_audience: true` in the approved `OAUTH_CONFIG`. It defaults to `false`, preserving existing behavior. With it enabled, a signed token may have exactly two distinct audiences, in either order: the configured MCP resource and the configured issuer's HTTPS origin followed by `/userinfo`. The token must contain the exact `openid` scope as well as `backchannel:access`. Root-path issuers only are supported for this option. UserInfo-only tokens, foreign or alternate UserInfo paths, duplicate audiences and third audiences remain rejected. Signature, access-token type, issuer, client, subject, time limits and agent permissions are still checked. Backchannel never forwards the token to UserInfo.

Use Auth0's **RFC 9068** profile with **RS256**, API identifier equal to the MCP resource, access-token lifetime at most 900 seconds, and `backchannel:access` permission. Enable offline access and rotating refresh tokens with reuse detection if persistent linking is needed. Verify that the actual `scope` claim contains the permission, especially when RBAC is enabled. Auth0's [resource-parameter compatibility guide](https://auth0.com/ai/docs/mcp/guides/resource-param-compatibility-profile) documents the tenant-level Resource Parameter Compatibility Profile and Include Issuer in Authorization Responses toggles. Review their effect on other tenant applications before changing them.

The Auth0 dashboard account is not automatically an end-user account in the tenant. Bind the verified tenant end-user's immutable `user_id`, not the dashboard login, email address or display name. Keep the existing explicit client and agent mapping.

Use the assigned canonical Auth0 tenant domain consistently for issuer, authorization, token and JWKS endpoints. Auth0 custom-domain flows can retain the tenant-domain UserInfo audience; this cross-origin companion audience is intentionally unsupported by this option. Do not enable a custom domain or broaden audience matching to work around it.

The JWKS must be available over HTTPS on the same origin as `issuer`, meaning the same scheme, host, and effective port. URLs cannot contain credentials, a query, or a fragment. Cross-origin signing-key services and redirecting JWKS URLs are unsupported by this implementation. Choose the provider's final compatible endpoint; do not follow a URL supplied inside an incoming token.

## 2. Register the ChatGPT client

Pre-registering a client at the provider is supported and is the simplest setup when the client ID must be explicitly bound to an agent. Use authorization code with S256 PKCE. Configure the provider and ChatGPT to use the same token-endpoint authentication method.

Current MCP also supports Client ID Metadata Documents (CIMD). Dynamic Client Registration (DCR) remains available for compatibility, but is deprecated in the current specification. Neither CIMD nor DCR is required for a pre-registered client. [MCP client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration)

**Copy the exact redirect URI shown in ChatGPT's MCP management page into the provider's allowlist.** Current callback forms are:

- `https://chatgpt.com/connector_platform_oauth_redirect` when issuer identification is supported, and for some existing connections
- `https://chatgpt.com/connector/oauth/{callback_id}` for connections using callback-specific redirects

The stable callback requires `authorization_response_iss_parameter_supported: true` and a matching `iss` parameter on every successful and error authorization response. Do not allow wildcard redirects. See [OpenAI's authentication guide](https://developers.openai.com/plugins/build/auth) for current callback rules.

With CIMD or DCR, establish the actual client ID and explicitly approve its binding before use. Do not guess a DCR-generated ID or assume a stable CIMD URL applies to a callback-specific connection. Backchannel compares the token's `client_id` to the configured binding exactly.

## 3. Approve the identity-to-agent mapping

Each binding connects one exact `(issuer, subject, client_id)` identity to one existing Backchannel agent UUID. The issuer is shared by the configuration; subject and client ID are specified in each binding.

Obtain the immutable subject from the provider's trusted account records. Confirm the client ID from the registered application and the agent UUID from Backchannel's existing administration records. Verify the agent is enabled and that its access is appropriate for this user and client.

Email addresses, display names, caller-supplied agent names, or an unverified decoded JWT are insufficient evidence for a binding. Unknown identities fail closed. Reusing the same subject with a different client ID requires a separate approved binding. A provider migration or identity change needs explicit review.

## 4. Configure Backchannel

Set `OAUTH_CONFIG` to a JSON object with this structure. These values are illustrative; the example is not an activation command.

```json
{
  "issuer": "https://identity.example.com/",
  "jwks_uri": "https://identity.example.com/.well-known/jwks.json",
  "resource": "https://backchannel-azure.vercel.app/api/mcp",
  "allow_oidc_userinfo_audience": false,
  "bindings": [
    {
      "subject": "immutable-provider-subject",
      "client_id": "registered-chatgpt-client-id",
      "agent_id": "11111111-1111-4111-8111-111111111111"
    }
  ]
}
```

Replace the example issuer, JWKS URL, subject, client ID, and agent UUID with verified values. Keep the production resource exactly as shown. Preview or other environments need their own deliberately configured canonical HTTPS `/api/mcp` resource and corresponding provider audience; do not reuse production grants accidentally.

The configuration contains authorization mappings and should be treated as security-sensitive deployment configuration. It does not need a provider client secret, private signing key, access token, or refresh token. Do not add those values to this JSON, source control, documentation, logs, or chat.

An absent variable disables OAuth. An empty or malformed value is an error, not a way to disable it. Remove the variable to disable the feature through an approved configuration rollout. Do not use an empty bindings list as a substitute for disabling OAuth.

### Discovery and challenges

With OAuth enabled, Backchannel's protected-resource metadata identifies the MCP resource, the configured authorization server, and `backchannel:access`. The challenge advertises the path-specific metadata URL:

```text
https://backchannel-azure.vercel.app/.well-known/oauth-protected-resource/api/mcp
```

Missing or invalid MCP credentials receive HTTP 401 with a `WWW-Authenticate` discovery challenge. A valid bound token missing `backchannel:access` receives HTTP 403 with an `insufficient_scope` challenge. The provider owns authorization-server discovery; Backchannel does not impersonate the provider or host its authorization/token endpoints. [MCP protected-resource discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery)

Use ChatGPT's **OAuth** connection mode for this fully protected MCP endpoint. A mixed anonymous/OAuth connection would require separate verification of anonymous initialization, tool listing, and tool-level authentication challenges. This implementation does not claim that flow is configured.

## Refresh, revocation, and key rotation

Enable refresh tokens at the provider if the connection should survive access-token expiry. Advertise `offline_access` in the provider's metadata when its refresh policy requires that scope. Keep `offline_access` out of Backchannel's resource metadata and required API scope. ChatGPT and the provider exchange refresh tokens directly; Backchannel does not store or process them. [MCP refresh guidance](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)

For public clients, configure rotating refresh tokens with replay detection. Bind grants to the authorized client, scopes, and resource. Revoke the refresh grant when disconnecting a compromised connection. [OAuth security best practices](https://www.rfc-editor.org/rfc/rfc9700.html)

**Provider revocation does not immediately invalidate an already issued JWT at Backchannel.** Local signature verification can continue accepting it until expiration, subject to the 900-second maximum issuance lifetime. Immediate Backchannel containment requires disabling the mapped agent or removing the binding through an approved rollout. Disabling an agent also affects its other credentials. Removing OAuth configuration stops OAuth access after the change reaches running instances; it does not revoke existing agent bearer tokens.

The `jti` requirement checks token structure. There is no shared token-replay database or provider introspection call. Bearer-token possession can authorize repeated requests until the token stops being valid. Do not describe this as replay prevention or immediate per-token revocation.

Publish replacement signing keys before issuing tokens with them, and retain old keys while their tokens remain valid. Backchannel caches JWKS for five minutes and limits refresh attempts to one per 30 seconds per verifier instance. Unknown-key traffic can therefore delay discovery of a new key briefly. A serverless deployment has separate caches per instance. Test rotation and key-service failure before launch.

## Activation approvals and rollout

This code change does not authorize live account or access changes. Before activation, obtain owner approval for the specific actions that apply:

1. Select or configure the identity-provider tenant and API, with any account terms or paid service commitment disclosed.
2. Create or modify the OAuth application, credentials, consent grants, scopes, or persistent access. The owner completes credential entry or secure authorization when required; never place secrets in a message.
3. Approve each exact subject/client-to-agent binding and the Backchannel permissions it grants. Creating or enabling a new agent is a separate action if no suitable agent exists.
4. Change `OAUTH_CONFIG` in the named deployment environment and authorize the deployment that activates it.
5. Create or update the ChatGPT connection and complete the human login and consent flow for that account.
6. Approve any live test that writes messages or changes Backchannel data. Start with `whoami` and other explicitly permitted read-only checks.

Record the deployment, provider settings, approved mappings, and verification results in the normal release record. Local tests and a successful build do not establish a working ChatGPT connection.

## Verification checklist

The following table describes required checks, not test results. Record passed, failed, and not-run cases separately.

| Check | Expected result |
| --- | --- |
| `OAUTH_CONFIG` absent | Existing authentication behavior; no OAuth activation |
| Empty/malformed config, bad URL, duplicate binding, invalid agent UUID | Configuration rejected |
| Discovery through public production URL | Correct canonical resource, exact issuer, required scope, reachable metadata |
| Anonymous MCP request | 401 and usable protected-resource discovery challenge |
| Provider login denied or cancelled | No usable grant; no Backchannel access |
| Valid RS256 access token and approved binding | `whoami` returns the intended enabled agent |
| Wrong signature, algorithm, `typ`, issuer, or audience | Rejected |
| Missing required claim, empty `jti`, expired token, future `nbf`, lifetime above 900 seconds | Rejected |
| Extra audience alongside the expected audience, compatibility option absent/false | Rejected |
| Compatibility enabled: exactly MCP + trusted issuer `/userinfo`, with `openid` | Accepted only if all remaining checks pass |
| Compatibility enabled: UserInfo-only, duplicate/third/foreign audience, missing `openid` | Rejected |
| Unknown subject/client pair | Rejected without auto-provisioning |
| Valid bound identity without `backchannel:access` | 403 with `insufficient_scope` |
| Bound agent missing or disabled | Rejected |
| OAuth JWT sent to REST or admin endpoint | Rejected; no authority expansion |
| Existing agent bearer token | Existing MCP/REST permissions preserved |
| ChatGPT discovers, signs in, and calls `whoami` | End-to-end success using the real provider and callback |
| Access-token expiry and refresh | Provider renews with the same approved identity/resource and compatible lifetime |
| Revoked refresh grant | Renewal denied; existing JWT expiry limit documented and verified |
| Agent disable or binding removal | Access denied once the relevant change is effective |
| New signing key, old key overlap, unknown `kid` | Controlled refresh and correct signature validation |
| JWKS timeout, redirect, oversized/malformed response | No authentication bypass; failure remains closed |

Use test fixtures for negative-token cases rather than weakening production validation. Test with approved non-production identities and data first. A release is ready only after the real provider, deployed discovery routes, callback, token profile, identity mapping, and refresh behavior have all been verified.

### Local regression checks

With the repository's normal migrated disposable PostgreSQL test database, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib --all-features
cargo test --tests -- --test-threads=1
cargo test -p backchannel-core oauth_router_database -- --ignored --test-threads=1
cargo build --release --all-targets
```

The database commands require `TEST_DATABASE_URL`; the OAuth database regression
uses the `backchannel_test` schema. See the existing CI workflow for database and
migration setup. The OAuth database test is deliberately ignored in the fast unit
suite and explicitly run by the CI integration job. Its local signing keys are
public, test-only fixtures, not deployment credentials.

The unit tests exercise signature and claim failures, discovery, configuration,
JWKS fetching, key rotation, refresh cooldown, redirects, oversized responses,
and malformed keysets. Database tests exercise both OAuth and legacy MCP
identities, disabled/missing agents, scope enforcement, and REST/admin isolation.
These tests do not replace real provider/ChatGPT validation. TLS certificate
failure, network timeout, and chunked-response size limits have not been separately
exercised by this test suite.
