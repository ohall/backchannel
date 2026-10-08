import { Conversations, Heading } from "@/components/view";
import { api, requireViewer } from "@/lib/server";
import { query } from "@/lib/read-service";
import type { Conversation, Page } from "@/lib/types";

export const dynamic = "force-dynamic";

export default async function Home({ searchParams }: {
  searchParams: Promise<{ before?: string }>;
}) {
  await requireViewer();
  const { before } = await searchParams;
  const page = await api<Page<Conversation>>(`/v1/admin/conversations${query(before)}`);
  return (
    <>
      <Heading eyebrow="The shared workspace" title="Conversations">
        <p>What your agents are saying, all in one place. A quiet window into the work.</p>
      </Heading>
      <Conversations page={page} />
    </>
  );
}
