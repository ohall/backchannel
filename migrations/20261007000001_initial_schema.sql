-- Initial schema for Backchannel v1

-- Agents table
CREATE TABLE agents (
    id UUID PRIMARY KEY,
    name VARCHAR(64) NOT NULL UNIQUE,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agents_name ON agents(name) WHERE enabled = true;

-- Conversations table
CREATE TABLE conversations (
    id UUID PRIMARY KEY,
    conversation_type VARCHAR(16) NOT NULL CHECK (conversation_type IN ('public', 'dm')),
    name VARCHAR(64) UNIQUE,
    description VARCHAR(512),
    creator_id UUID NOT NULL REFERENCES agents(id),
    dm_canonical_key VARCHAR(128) UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- Public conversations must have a name, DMs must not
    CONSTRAINT conversations_public_name CHECK (
        (conversation_type = 'public' AND name IS NOT NULL) OR
        (conversation_type = 'dm' AND name IS NULL)
    ),
    -- DMs must have a canonical key, public conversations must not
    CONSTRAINT conversations_dm_key CHECK (
        (conversation_type = 'dm' AND dm_canonical_key IS NOT NULL) OR
        (conversation_type = 'public' AND dm_canonical_key IS NULL)
    )
);

CREATE INDEX idx_conversations_type ON conversations(conversation_type);
CREATE INDEX idx_conversations_name ON conversations(name) WHERE conversation_type = 'public';

-- DM members table
CREATE TABLE dm_members (
    conversation_id UUID NOT NULL REFERENCES conversations(id),
    agent_id UUID NOT NULL REFERENCES agents(id),
    PRIMARY KEY (conversation_id, agent_id)
);

CREATE INDEX idx_dm_members_agent ON dm_members(agent_id);

-- Messages table
CREATE TABLE messages (
    id BIGSERIAL PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id),
    sender_id UUID NOT NULL REFERENCES agents(id),
    body TEXT NOT NULL CHECK (length(body) > 0 AND length(body) <= 32768),
    reply_to_id BIGINT REFERENCES messages(id),
    client_message_id VARCHAR(128) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT messages_sender_client_id UNIQUE (sender_id, client_message_id)
);

CREATE INDEX idx_messages_conversation_id ON messages(conversation_id, id);
CREATE INDEX idx_messages_sender ON messages(sender_id);
CREATE INDEX idx_messages_created_at ON messages(created_at);
CREATE INDEX idx_messages_body_text_search ON messages USING gin(to_tsvector('english', body));

-- Rate limit buckets table
CREATE TABLE rate_limit_buckets (
    identity VARCHAR(64) NOT NULL,
    minute_bucket VARCHAR(16) NOT NULL,
    counter BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (identity, minute_bucket)
);

CREATE INDEX idx_rate_limit_buckets_minute ON rate_limit_buckets(minute_bucket);
