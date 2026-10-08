import { describe, expect, it, vi } from "vitest";
import { query, readData, type ReadDependencies } from "../lib/read-service";
const now = Date.now();
const good = { user: { email: "oakley349@gmail.com", email_verified: true }, internal: { createdAt: now / 1000 - 10 } };
function dependencies(): ReadDependencies { return { session: async () => good, allowedEmails: "oakley349@gmail.com", baseUrl: "https://api.example.test", viewerToken: "synthetic-viewer-token", fetcher: vi.fn(async () => Response.json({ items: [], has_more: false, next_cursor: null })), now: () => now }; }
describe("authenticated read paths", () => {
    it.each([null, { ...good, user: { email: "other@example.com", email_verified: true } }, { ...good, user: { email: "oakley349@gmail.com", email_verified: false } }, { ...good, internal: { createdAt: 0 } }])("rejects before any fetch", async (session) => { const deps = dependencies(); deps.session = async () => session; await expect(readData("/v1/admin/agents", deps)).rejects.toMatchObject({ status: 403 }); expect(deps.fetcher).not.toHaveBeenCalled(); });
    it.each(["/v1/admin/conversations", "/v1/admin/agents", "/v1/admin/conversations/abcd-1234/messages", "/v1/admin/search?search=hello"])("reads explicit endpoint %s without caching", async (path) => { const deps = dependencies(); await readData(path, deps); expect(deps.fetcher).toHaveBeenCalledWith(expect.any(URL), expect.objectContaining({ method: "GET", cache: "no-store", redirect: "error", headers: { Authorization: "Bearer synthetic-viewer-token", Accept: "application/json" } })); });
    it.each(["/api/mcp", "https://evil.test", "/v1/admin/export", "/v1/admin/agents/rotate"])("rejects unrelated endpoint %s", async (path) => { const deps = dependencies(); await expect(readData(path, deps)).rejects.toMatchObject({ status: 400 }); expect(deps.fetcher).not.toHaveBeenCalled(); });
    it("never falls back to admin credentials", async () => { const deps = dependencies(); deps.viewerToken = undefined; await expect(readData("/v1/admin/agents", deps)).rejects.toMatchObject({ status: 503 }); expect(deps.fetcher).not.toHaveBeenCalled(); });
    it("does not reflect upstream or token errors", async () => { const deps = dependencies(); deps.fetcher = vi.fn(async () => { throw Error("synthetic-viewer-token secret upstream body"); }); await expect(readData("/v1/admin/agents", deps)).rejects.toThrow("Backchannel could not be reached. Please try again."); });
    it.each(["http://remote.test", "https://user:pass@api.example.test", "https://api.example.test/path", "not-a-url"])("rejects unsafe base %s", async (base) => { const deps = dependencies(); deps.baseUrl = base; await expect(readData("/v1/admin/agents", deps)).rejects.toMatchObject({ status: 503 }); expect(deps.fetcher).not.toHaveBeenCalled(); });
});
describe("pagination and search", () => {
    it.each(["m1:123", "c1:00000000-0000-0000-0000-000000000001", "a1:00000000-0000-0000-0000-000000000001"])("round-trips API cursor %s", cursor => expect(new URLSearchParams(query(cursor)).get("before")).toBe(cursor));
    it("encodes query text as data", () => expect(new URLSearchParams(query(undefined, "a&limit=100")).get("search")).toBe("a&limit=100"));
    it("bounds cursors and queries", () => { expect(() => query("x".repeat(65))).toThrow(); expect(() => query(undefined, "x".repeat(257))).toThrow(); expect(() => query("../")).toThrow(); expect(() => query(undefined, "界".repeat(86))).toThrow(); expect(() => query(undefined, "界".repeat(85))).not.toThrow(); });
});
