# Backchannel viewer

Private, read-only Next.js App Router application. Auth0 handles login/session cookies; every protected page and every server-side API request independently checks the exact verified owner identity. No write controls or browser-facing API-token endpoint exist.

## Local development

Use Node 24 and npm. After installing approved development dependencies:

```sh
npm ci
cp .env.example .env.local
# Populate approved local configuration securely; do not commit real values.
npm run dev
```

Only `ALLOWED_EMAILS=oakley349@gmail.com` enables the specified owner. Empty configuration denies all access, and adding another address does not broaden this viewer's fixed owner policy. Auth0 must issue `email` and boolean `email_verified` claims. Dashboard access alone does not create an end-user identity.

`BACKCHANNEL_VIEWER_TOKEN` must be the dedicated read-only credential. There is deliberately no admin-token fallback. `BACKCHANNEL_API_URL` is an HTTPS origin, or an HTTP loopback origin for local development. The token never crosses into browser code. No `NEXT_PUBLIC_*` secrets are used.

Configure the Auth0 Regular Web Application callback as the approved `APP_BASE_URL` plus `/auth/callback`, and approve the base URL and `/login` logout return URL as needed by Auth0. Never register a guessed production domain. The SDK uses `/auth/login`, `/auth/logout`, and `/auth/callback`; all other SDK HTTP endpoints are blocked. Safe return paths are limited to this viewer's pages.

Sessions are nonrolling and expire after eight hours. Auth0 validates encrypted session expiry; the application separately validates session creation/absolute age and owner authorization. All protected rendering is dynamic, upstream fetches are `no-store`, and responses request private/no-store caching. Navigation uses regular document links, avoiding persistent client-router caches. Restored history documents are hidden before revalidation and the client revalidates at absolute expiry.

## Checks

```sh
npm test             # Node built-in runner: security/data boundary regression tests
npm run test:ui      # Vitest: server-rendered UI, auth, fetch, pagination, DST
npm run typecheck
npm run lint
npm run build
npm run test:http    # Local production server + synthetic encrypted SDK sessions
```

The core tests inject session and fetch dependencies into pure functions. This is not a production login bypass. Any synthetic browser fixture harness must live outside the production app. No production API or real credential is needed for these checks.

Pages include conversations/DMs, messages with load-older navigation, public agent metadata, and message search. Time labels use `America/New_York` with EDT/EST, preserving UTC values in `time` attributes. Message bodies are React-escaped plain text, with no Markdown/HTML rendering or automatic URL execution. Search is bounded to 256 UTF-8 bytes and cursors to 64 characters, matching the API. The conversation detail view currently uses a generic title because the API does not return conversation metadata with messages.

## Deployment boundary

Deployment is a separate approved action. Use a separate Vercel project rooted at `web/`, after approved Auth0 application setup and secure environment provisioning. `vercel.json` disables automatic deployments for the local review branch `feat/readonly-viewer`. Do not enable deployments or publish a different branch without the corresponding guard and approval.

See `../onboarding/VIEWER.md` for provider setup and the remaining live acceptance checks. Local tests do not verify actual Auth0 login, rejection of a real second account, or production Backchannel access.
