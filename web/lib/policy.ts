export const SESSION_SECONDS = 8 * 60 * 60;
export type Identity = {
    user?: {
        email?: unknown;
        email_verified?: unknown;
    };
    internal?: {
        createdAt?: unknown;
    };
} | null | undefined;
export function isAllowed(session: Identity, allowlist: string | undefined, now = Date.now()): boolean {
    const email = session?.user?.email;
    const created = session?.internal?.createdAt;
    if (typeof email !== "string" || session?.user?.email_verified !== true || typeof created !== "number" || !Number.isFinite(created))
        return false;
    if (created * 1000 > now || (created + SESSION_SECONDS) * 1000 <= now)
        return false;
    // This viewer is explicitly scoped to Oakley. Configuration can disable access,
    // but cannot silently broaden it to another identity.
    return email === "oakley349@gmail.com" && (allowlist ?? "").split(",").map(s => s.trim()).includes(email);
}
export function safeReturnPath(value: unknown): string {
    if (typeof value !== "string" || /[\\\r\n\u0000]/.test(value))
        return "/";
    if (!/^\/(?:$|agents(?:\?|$)|search(?:\?|$)|conversations\/[0-9a-f-]+(?:\?|$))/i.test(value))
        return "/";
    return value;
}
