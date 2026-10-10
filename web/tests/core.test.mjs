// Core security regression tests need only Node 24. No test-only auth is mounted in Next.
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import { test } from "node:test";
import assert from "node:assert/strict";
const load=async path=>stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),"utf8"),{mode:"transform"});
const data=source=>`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
const policyUrl=data(await load("../lib/policy.ts"));
const {isAllowed,safeReturnPath,SESSION_SECONDS}=await import(policyUrl);
const {readData,query}=await import(data((await load("../lib/read-service.ts")).replace('"./policy"',JSON.stringify(policyUrl))));
const {timestamp}=await import(data(await load("../lib/format.ts")));
const now=Date.parse("2026-10-08T12:00:00Z");
const good={user:{email:"oakley349@gmail.com",email_verified:true},internal:{createdAt:now/1000-60}};
function deps(session=good){const calls=[];return {calls,session:async()=>session,allowedEmails:"oakley349@gmail.com",baseUrl:"https://api.example.test",viewerToken:"synthetic-viewer-token",now:()=>now,fetcher:async(...args)=>{calls.push(args);return Response.json({items:[],next_cursor:null,has_more:false});}};}
test("verified configured owner succeeds",()=>assert.equal(isAllowed(good,"oakley349@gmail.com",now),true));
for(const [name,session] of Object.entries({missing:null,empty:{},wrong:{...good,user:{email:"other@example.com",email_verified:true}},unverified:{...good,user:{email:"oakley349@gmail.com",email_verified:false}},stringClaim:{...good,user:{email:"oakley349@gmail.com",email_verified:"true"}},missingEmail:{...good,user:{email_verified:true}},expired:{...good,internal:{createdAt:now/1000-SESSION_SECONDS}},future:{...good,internal:{createdAt:now/1000+1}},invalidDate:{...good,internal:{createdAt:NaN}}}))test(`denies ${name} before fetch`,async()=>{const d=deps(session);await assert.rejects(readData("/v1/admin/agents",d),{status:403});assert.equal(d.calls.length,0);});
test("missing allowlist denies",()=>assert.equal(isAllowed(good,undefined,now),false));
for(const path of ["https://evil.test","//evil.test","/\\evil.test","/%2f%2fevil.test","/auth/logout","/search\nLocation: bad"])test(`reject return ${JSON.stringify(path)}`,()=>assert.equal(safeReturnPath(path),"/"));
for(const path of ["/","/agents","/search?search=hello","/conversations/abcd-1234?before=m1%3A12"])test(`safe return ${path}`,()=>assert.equal(safeReturnPath(path),path));
for(const path of ["/v1/admin/conversations","/v1/admin/agents","/v1/admin/conversations/abcd-1234/messages","/v1/admin/search?search=word"])test(`read-only request ${path}`,async()=>{const d=deps();await readData(path,d);assert.equal(d.calls.length,1);const [url,opts]=d.calls[0];assert.equal(url.origin,"https://api.example.test");assert.equal(opts.method,"GET");assert.equal(opts.cache,"no-store");assert.equal(opts.redirect,"error");assert.equal(opts.headers.Authorization,"Bearer synthetic-viewer-token");});
for(const path of ["/api/mcp","https://evil.test","/v1/admin/export","/v1/admin/agents/rotate"])test(`reject endpoint ${path}`,async()=>{const d=deps();await assert.rejects(readData(path,d),{status:400});assert.equal(d.calls.length,0);});
test("no admin fallback",async()=>{const d=deps();d.viewerToken=undefined;await assert.rejects(readData("/v1/admin/agents",d),{status:503});assert.equal(d.calls.length,0);});
test("error redaction",async()=>{const d=deps();d.fetcher=async()=>{throw Error("secret synthetic-viewer-token");};await assert.rejects(readData("/v1/admin/agents",d),{message:"Backchannel could not be reached. Please try again."});});
for(const base of ["http://remote.test","https://user:pass@api.example.test","https://api.example.test/path","broken"])test(`reject base ${base}`,async()=>{const d=deps();d.baseUrl=base;await assert.rejects(readData("/v1/admin/agents",d),{status:503});assert.equal(d.calls.length,0);});
for(const cursor of ["m1:123","c1:00000000-0000-0000-0000-000000000001","a1:00000000-0000-0000-0000-000000000001"])test(`opaque pagination ${cursor}`,()=>assert.equal(new URLSearchParams(query(cursor)).get("before"),cursor));
test("bounds UTF-8 search and cursor",()=>{assert.throws(()=>query("x".repeat(65)));assert.throws(()=>query("../"));assert.throws(()=>query(undefined,"界".repeat(86)));assert.doesNotThrow(()=>query(undefined,"界".repeat(85)));assert.equal(new URLSearchParams(query(undefined,"a&limit=100")).get("search"),"a&limit=100");});
test("Eastern timezone respects DST",()=>{assert.match(timestamp("2026-07-01T12:00:00Z"),/8:00 AM EDT/);assert.match(timestamp("2026-01-01T12:00:00Z"),/7:00 AM EST/);});
