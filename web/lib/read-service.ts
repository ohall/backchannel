import { isAllowed, type Identity } from "./policy";
export class ReadError extends Error {
    constructor(public readonly status: number, message: string) { super(message); }
}
export type ReadDependencies = {
    session: () => Promise<Identity>;
    allowedEmails: string | undefined;
    baseUrl: string | undefined;
    viewerToken: string | undefined;
    fetcher: typeof fetch;
    now?: () => number;
};
// Dependency injection is only a unit-test seam. Production always supplies Auth0.
export async function readData<T>(path: string, deps: ReadDependencies): Promise<T> {
    if (!isAllowed(await deps.session(), deps.allowedEmails, deps.now?.()))
        throw new ReadError(403, "Sign in with the approved, verified account.");
    if (!/^\/v1\/admin\/(conversations(?:\/[0-9a-f-]+\/messages)?|agents|search)(?:\?|$)/i.test(path))
        throw new ReadError(400, "Invalid request.");
    if (!deps.baseUrl || !deps.viewerToken)
        throw new ReadError(503, "The viewer is not configured yet.");
    let url: URL;
    try {
        const base = new URL(deps.baseUrl);
        if (base.username || base.password || base.search || base.hash || base.pathname !== "/")
            throw new Error();
        if (base.protocol !== "https:" && !(base.protocol === "http:" && ["localhost", "127.0.0.1"].includes(base.hostname)))
            throw new Error();
        url = new URL(path, base);
    }
    catch {
        throw new ReadError(503, "The viewer is not configured yet.");
    }
    try {
        const response = await deps.fetcher(url, { method: "GET", headers: { Authorization: `Bearer ${deps.viewerToken}`, Accept: "application/json" }, cache: "no-store", redirect: "error", signal: AbortSignal.timeout(10000) });
        if (!response.ok)
            throw new Error();
        return await response.json() as T;
    }
    catch {
        throw new ReadError(502, "Backchannel could not be reached. Please try again.");
    }
}
export function query(before?: string, search?: string): string {
    const params = new URLSearchParams({ limit: "50" });
    if (before) {
        if (before.length > 64 || !/^[A-Za-z0-9_:-]+$/.test(before))
            throw new ReadError(400, "Invalid page cursor.");
        params.set("before", before);
    }
    if (search) {
        if (new TextEncoder().encode(search).length > 256)
            throw new ReadError(400, "Search must be 256 UTF-8 bytes or fewer.");
        params.set("search", search);
    }
    return `?${params}`;
}
