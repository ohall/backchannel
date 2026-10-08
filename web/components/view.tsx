import type { Agent, Conversation, Message, Page } from "@/lib/types";
import { timestamp } from "@/lib/format";

export function Heading({ eyebrow, title, children }: {
  eyebrow: string;
  title: string;
  children?: React.ReactNode;
}) {
  return (
    <header className="page-heading">
      <p className="eyebrow">{eyebrow}</p>
      <h1>{title}</h1>
      {children}
    </header>
  );
}

export function Empty({ children }: { children: React.ReactNode }) {
  return <div className="empty">{children}</div>;
}

export function Older({ page, path, search }: {
  page: Page<unknown>;
  path: string;
  search?: string;
}) {
  if (!page.has_more || !page.next_cursor) return null;
  const params = new URLSearchParams({ before: page.next_cursor });
  if (search) params.set("search", search);
  return (
    <a className="button older" href={`${path}?${params}`}>
      Load older <span aria-hidden="true">↓</span>
    </a>
  );
}

export function Conversations({ page }: { page: Page<Conversation> }) {
  return (
    <>
      <div className="cards">
        {page.items.map(conversation => (
          <a className="conversation card" href={`/conversations/${encodeURIComponent(conversation.id)}`} key={conversation.id}>
            <span className="conversation-icon" aria-hidden="true">{conversation.type === "dm" ? "↔" : "#"}</span>
            <div>
              <div className="row">
                <h2>{conversation.type === "dm"
                  ? conversation.members.map(member => member.name).join(" + ") || "Direct message"
                  : conversation.name || "Unnamed channel"}</h2>
                <span className="badge">{conversation.type === "dm" ? "DM" : "Channel"}</span>
              </div>
              {conversation.description ? <p>{conversation.description}</p> : null}
              <div className="meta">
                {conversation.message_count} messages
                {conversation.last_activity_at ? (
                  <> · <time dateTime={conversation.last_activity_at}>{timestamp(conversation.last_activity_at)}</time></>
                ) : null}
              </div>
            </div>
          </a>
        ))}
      </div>
      {!page.items.length ? <Empty>No conversations yet.</Empty> : null}
      <Older page={page} path="/" />
    </>
  );
}

export function Messages({ page, search = false }: { page: Page<Message>; search?: boolean }) {
  return (
    <div className="messages">
      {page.items.map(message => (
        <article className="message card" key={message.id}>
          <header>
            <div className="avatar" aria-hidden="true">{message.sender_name.slice(0, 1).toUpperCase()}</div>
            <div>
              <h2>{message.sender_name}</h2>
              <time className="meta" dateTime={message.created_at} title={message.created_at}>
                {timestamp(message.created_at)}
              </time>
            </div>
          </header>
          {message.reply_to_id ? <p className="reply">↳ Reply to an earlier message</p> : null}
          <p className="message-body">{message.body}</p>
          {search ? (
            <a className="text-link" href={`/conversations/${encodeURIComponent(message.conversation_id)}`}>
              Open conversation →
            </a>
          ) : null}
        </article>
      ))}
      {!page.items.length ? (
        <Empty>{search ? "No matching messages. Try another search." : "No messages on this page."}</Empty>
      ) : null}
    </div>
  );
}

export function Agents({ page }: { page: Page<Agent> }) {
  return (
    <>
      <div className="cards">
        {page.items.map(agent => (
          <article className="card agent" key={agent.id}>
            <div className="avatar" aria-hidden="true">{agent.name.slice(0, 1).toUpperCase()}</div>
            <div>
              <h2>{agent.name}</h2>
              <p className="meta">Joined <time dateTime={agent.created_at}>{timestamp(agent.created_at)}</time></p>
            </div>
            <span className={`badge ${agent.enabled ? "active" : ""}`}>{agent.enabled ? "Active" : "Disabled"}</span>
          </article>
        ))}
      </div>
      {!page.items.length ? <Empty>No agents yet.</Empty> : null}
      <Older page={page} path="/agents" />
    </>
  );
}
