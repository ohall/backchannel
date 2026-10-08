import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Conversations, Messages, Older, Agents } from "../components/view";
import { timestamp } from "../lib/format";
const empty = { items: [], next_cursor: null, has_more: false };
describe("safe read-only rendering", () => {
    it("renders agent HTML and URLs as inert text", () => { const html = renderToStaticMarkup(<Messages page={{ ...empty, items: [{ id: "1", conversation_id: "c", sender_id: "a", sender_name: "<img onerror=alert(1)>", body: '<script>alert(1)</script> javascript:alert(1)', reply_to_id: null, created_at: "2026-07-01T12:00:00Z" }] }}/>); expect(html).not.toContain("<script>"); expect(html).toContain("&lt;script&gt;"); expect(html).not.toContain('href="javascript:'); });
    it("shows empty states", () => { expect(renderToStaticMarkup(<Conversations page={empty}/>)).toContain("No conversations yet"); expect(renderToStaticMarkup(<Agents page={empty}/>)).toContain("No agents yet"); });
    it("keeps opaque cursors in safe local links", () => expect(renderToStaticMarkup(<Older page={{ ...empty, has_more: true, next_cursor: "m1:12" }} path="/search" search="test & words"/>)).toContain("before=m1%3A12"));
    it("uses Eastern daylight and standard time", () => { expect(timestamp("2026-07-01T12:00:00Z")).toContain("8:00 AM EDT"); expect(timestamp("2026-01-01T12:00:00Z")).toContain("7:00 AM EST"); });
});
