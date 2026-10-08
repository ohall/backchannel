import { describe, expect, it } from "vitest";
import { isAllowed, safeReturnPath, SESSION_SECONDS } from "../lib/policy";
const now = Date.parse("2026-10-08T12:00:00Z");
const session = { user: { email: "oakley349@gmail.com", email_verified: true }, internal: { createdAt: now / 1000 - 60 } };
describe("viewer allowlist", () => {
    it("allows only the verified configured owner", () => expect(isAllowed(session, "oakley349@gmail.com", now)).toBe(true));
    it.each([null, {}, { ...session, user: { email: "other@example.com", email_verified: true } }, { ...session, user: { email: "oakley349@gmail.com", email_verified: false } }, { ...session, user: { email: "oakley349@gmail.com", email_verified: "true" } }, { ...session, user: { email_verified: true } }, { ...session, internal: { createdAt: now / 1000 - SESSION_SECONDS } }, { ...session, internal: { createdAt: now / 1000 + 60 } }, { ...session, internal: { createdAt: NaN } }])("denies invalid identity or expired session", value => expect(isAllowed(value, "oakley349@gmail.com,other@example.com", now)).toBe(false));
    it("fails closed without allowlist", () => expect(isAllowed(session, undefined, now)).toBe(false));
});
describe("return paths", () => {
    it.each(["https://evil.test", "//evil.test", "/\\evil.test", "/%2f%2fevil.test", "/auth/logout", "/search\nLocation: evil"])('rejects %s', value => expect(safeReturnPath(value)).toBe("/"));
    it.each(["/", "/agents", "/search?search=word", "/conversations/abcd-1234?before=m1%3A12"])('permits %s', value => expect(safeReturnPath(value)).toBe(value));
});
