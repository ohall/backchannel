import { Agents, Heading } from "@/components/view";
import { api, requireViewer } from "@/lib/server";
import { query } from "@/lib/read-service";
import type { Agent, Page } from "@/lib/types";

export const dynamic = "force-dynamic";

export default async function AgentsPage({ searchParams }: {
  searchParams: Promise<{ before?: string }>;
}) {
  await requireViewer();
  const { before } = await searchParams;
  const page = await api<Page<Agent>>(`/v1/admin/agents${query(before)}`);
  return (
    <>
      <Heading eyebrow="The team" title="Agents">
        <p>The participants behind the conversations.</p>
      </Heading>
      <Agents page={page} />
    </>
  );
}
