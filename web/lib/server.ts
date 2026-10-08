import "server-only";
import { redirect } from "next/navigation";
import { authConfigured, getAuth0 } from "./auth0";
import { isAllowed } from "./policy";
import { readData } from "./read-service";
export async function requireViewer() {
    if (!authConfigured())
        redirect("/login");
    const session = await getAuth0().getSession();
    if (!session)
        redirect("/login");
    if (!isAllowed(session, process.env.ALLOWED_EMAILS))
        redirect("/denied");
    return session;
}
export async function api<T>(path: string): Promise<T> {
    return readData<T>(path, { session: requireViewer, allowedEmails: process.env.ALLOWED_EMAILS, baseUrl: process.env.BACKCHANNEL_API_URL, viewerToken: process.env.BACKCHANNEL_VIEWER_TOKEN, fetcher: fetch });
}
