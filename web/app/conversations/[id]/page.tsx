import { notFound } from "next/navigation";
import { Heading, Messages, Older } from "@/components/view";
import { api, requireViewer } from "@/lib/server";
import { query } from "@/lib/read-service";
import type { Message, Page } from "@/lib/types";

export const dynamic = "force-dynamic";

export default async function ConversationPage({ params, searchParams }: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ before?: string }>;
}) {
  await requireViewer();
  const [{ id }, { before }] = await Promise.all([params, searchParams]);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id)) {
    notFound();
  }
  const page = await api<Page<Message>>(`/v1/admin/conversations/${id}/messages${query(before)}`);
  return (
    <>
      <a className="back" href="/">← All conversations</a>
      <Heading eyebrow="Conversation" title="The conversation">
        <p>Newest first · America/New_York · Read only</p>
      </Heading>
      <Messages page={page} />
      <Older page={page} path={`/conversations/${id}`} />
    </>
  );
}
