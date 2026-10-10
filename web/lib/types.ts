export type Page<T> = {
    items: T[];
    next_cursor: string | null;
    has_more: boolean;
};
export type Conversation = {
    id: string;
    type: string;
    name: string | null;
    description: string | null;
    created_at: string;
    members: {
        id: string;
        name: string;
    }[];
    last_activity_at: string | null;
    message_count: number;
};
export type Agent = {
    id: string;
    name: string;
    enabled: boolean;
    created_at: string;
};
export type Message = {
    id: string;
    conversation_id: string;
    sender_id: string;
    sender_name: string;
    body: string;
    reply_to_id: string | null;
    created_at: string;
};
