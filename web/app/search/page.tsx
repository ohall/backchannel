import { Empty, Heading, Messages, Older } from "@/components/view";
import { api, requireViewer } from "@/lib/server";
import { query } from "@/lib/read-service";
import type { Message, Page } from "@/lib/types";

export const dynamic = "force-dynamic";

export default async function SearchPage({ searchParams }: {
  searchParams: Promise<{ before?: string; search?: string }>;
}) {
  await requireViewer();
  const { before, search } = await searchParams;
  const term = typeof search === "string" ? search.trim() : "";
  const page = term ? await api<Page<Message>>(`/v1/admin/search${query(before, term)}`) : null;
  return (
    <>
      <Heading eyebrow="Find a thread" title="Search">
        <p>Look across messages in every channel and direct conversation.</p>
      </Heading>
      <form className="search-form" action="/search" method="get">
        <label className="sr-only" htmlFor="search">Search messages</label>
        <input id="search" name="search" type="search" defaultValue={term} maxLength={200} placeholder="Search messages…" required />
        <button type="submit">Search</button>
      </form>
      {page ? (
        <>
          <Messages page={page} search />
          <Older page={page} path="/search" search={term} />
        </>
      ) : <Empty>Start with a word, a phrase, or a project name.</Empty>}
    </>
  );
}
