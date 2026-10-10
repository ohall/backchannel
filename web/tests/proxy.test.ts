import { beforeEach, describe, expect, it, vi } from "vitest";
import { NextRequest, NextResponse } from "next/server";
import { Auth0Client } from "@auth0/nextjs-auth0/server";
const fixture = vi.hoisted(() => ({ auth: null as unknown as Auth0Client }));
vi.mock("../lib/auth0", () => ({ authConfigured: () => true, getAuth0: () => fixture.auth }));
import { proxy } from "../proxy";
beforeEach(() => { process.env.APP_BASE_URL = "https://viewer.example.test"; process.env.ALLOWED_EMAILS = "oakley349@gmail.com"; });
describe("provider logout", () => {
    it.each(["v2", "oidc"] as const)("uses an absolute fixed target with %s and clears the local cookie", async strategy => {
        fixture.auth = new Auth0Client({ domain: "fixture.auth0.invalid", clientId: "synthetic", clientSecret: "synthetic", secret: "ab".repeat(32), appBaseUrl: process.env.APP_BASE_URL, logoutStrategy: strategy,
            customFetch: async () => Response.json({ issuer: "https://fixture.auth0.invalid/", authorization_endpoint: "https://fixture.auth0.invalid/authorize", token_endpoint: "https://fixture.auth0.invalid/oauth/token", jwks_uri: "https://fixture.auth0.invalid/.well-known/jwks.json", end_session_endpoint: "https://fixture.auth0.invalid/oidc/logout" }),
        });
        const response = await proxy(new NextRequest("https://attacker-host.test/auth/logout?returnTo=https://evil.test"));
        const location = new URL(response.headers.get("location")!);
        expect(location.origin).toBe("https://fixture.auth0.invalid");
        expect(location.searchParams.get(strategy === "v2" ? "returnTo" : "post_logout_redirect_uri")).toBe("https://viewer.example.test/login");
        expect(response.headers.get("set-cookie")).toMatch(/__session=.*Max-Age=0/);
        expect(response.headers.get("cache-control")).toContain("no-store");
    });
});
describe("SDK route containment", () => {
    it.each(["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"])("never delegates unused proxies for %s", async method => {
        const middleware = vi.fn(); fixture.auth = { middleware, getSession: vi.fn() } as unknown as Auth0Client;
        for (const path of ["/me", "/me/v1/profile", "/my-org", "/my-org/organizations/example", "/auth/access-token", "/auth/profile"]) {
            expect((await proxy(new NextRequest(`https://viewer.example.test${path}`, { method }))).status).toBe(404);
        }
        expect(middleware).not.toHaveBeenCalled();
    });
    it("continues owner pages without invoking SDK middleware", async () => {
        const middleware = vi.fn();
        fixture.auth = { middleware, getSession: vi.fn(async () => ({ user: { email: "oakley349@gmail.com", email_verified: true }, internal: { createdAt: Date.now() / 1000 - 60 } })) } as unknown as Auth0Client;
        const response = await proxy(new NextRequest("https://viewer.example.test/agents"));
        expect(response.headers.get("x-middleware-next")).toBe("1"); expect(middleware).not.toHaveBeenCalled();
    });
    it("rejects auth mutation methods before SDK dispatch", async () => {
        const middleware = vi.fn(async () => NextResponse.next()); fixture.auth = { middleware } as unknown as Auth0Client;
        expect((await proxy(new NextRequest("https://viewer.example.test/auth/logout", { method: "POST" }))).status).toBe(405);
        expect(middleware).not.toHaveBeenCalled();
    });
});
