import { NextRequest, NextResponse } from "next/server";
import { authConfigured, getAuth0 } from "./lib/auth0";
import { isAllowed, safeReturnPath } from "./lib/policy";
const noStore = (response: NextResponse) => {
    response.headers.set("Cache-Control", "private, no-store, max-age=0");
    return response;
};
export async function proxy(request: NextRequest) {
    const path = request.nextUrl.pathname;
    const publicPage = path === "/login" || path === "/denied";
    if (!authConfigured())
        return noStore(publicPage ? NextResponse.next() : NextResponse.redirect(new URL("/login", request.url)));
    const auth = getAuth0();
    // Only the SDK's login, callback and logout routes are needed by this viewer.
    if (path.startsWith("/auth/")) {
        if (!["/auth/login", "/auth/callback", "/auth/logout"].includes(path))
            return noStore(new NextResponse(null, { status: 404 }));
        if (path === "/auth/login" || path === "/auth/logout") {
            const url = request.nextUrl.clone();
            url.searchParams.set("returnTo", path === "/auth/logout" ? "/login" : safeReturnPath(url.searchParams.get("returnTo")));
            return noStore(await auth.middleware(new NextRequest(url, request)));
        }
        return noStore(await auth.middleware(request));
    }
    if (publicPage)
        return noStore(await auth.middleware(request));
    const session = await auth.getSession(request);
    if (!session)
        return noStore(NextResponse.redirect(new URL("/login", request.url)));
    if (!isAllowed(session, process.env.ALLOWED_EMAILS))
        return noStore(NextResponse.redirect(new URL("/denied", request.url)));
    return noStore(await auth.middleware(request));
}
export const config = { matcher: ["/((?!_next/static|_next/image|favicon.ico).*)"] };
