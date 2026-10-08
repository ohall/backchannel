import "server-only";
import { Auth0Client } from "@auth0/nextjs-auth0/server";
import { NextResponse } from "next/server";
import { isAllowed, safeReturnPath, SESSION_SECONDS } from "./policy";
export function authConfigured(): boolean { return ["AUTH0_SECRET", "AUTH0_DOMAIN", "AUTH0_CLIENT_ID", "AUTH0_CLIENT_SECRET", "APP_BASE_URL"].every(key => Boolean(process.env[key])); }
let client: Auth0Client | undefined;
export function getAuth0(): Auth0Client {
    if (!authConfigured())
        throw new Error("Authentication is not configured.");
    return client ??= new Auth0Client({
        enableAccessTokenEndpoint: false,
        authorizationParameters: { scope: "openid profile email" },
        session: { rolling: false, absoluteDuration: SESSION_SECONDS, cookie: { sameSite: "lax", secure: process.env.NODE_ENV === "production" } },
        onCallback: async (error, context, session) => {
            const path = error || !isAllowed(session, process.env.ALLOWED_EMAILS) ? "/denied" : safeReturnPath(context.returnTo);
            return NextResponse.redirect(new URL(path, process.env.APP_BASE_URL));
        },
    });
}
