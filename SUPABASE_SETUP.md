# Supabase Setup for Backchannel

## Project Information

- **Project**: backchannel
- **Reference**: arfxachrbugnvbneyboe
- **Region**: us-east-1 (AWS)
- **Plan**: Free tier
- **Database**: Single Supabase project with schema isolation

## Multi-Environment Strategy

Since the free plan allows only 2 active projects, we use **schema isolation** within a single project:

- **Production**: `backchannel` schema
- **Preview**: `backchannel_preview` schema

Each environment has its own set of roles with access only to its schema.

## Role and Schema Setup

### 1. Connect as Postgres Superuser

Get the postgres password from Supabase Dashboard:
- Project Settings > Database > Database password

```bash
psql "postgresql://postgres.arfxachrbugnvbneyboe:[YOUR_PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require"
```

### 2. Create Schemas

```sql
-- Create production schema
CREATE SCHEMA IF NOT EXISTS backchannel;

-- Create preview schema  
CREATE SCHEMA IF NOT EXISTS backchannel_preview;
```

### 3. Create Migration Roles

These roles have DDL privileges to run migrations:

```sql
-- Production migration role
CREATE ROLE backchannel_migrate WITH LOGIN PASSWORD 'CHANGE_ME_STRONG_PASSWORD_1';
GRANT USAGE ON SCHEMA backchannel TO backchannel_migrate;
GRANT CREATE ON SCHEMA backchannel TO backchannel_migrate;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel 
    GRANT ALL ON TABLES TO backchannel_migrate;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel 
    GRANT ALL ON SEQUENCES TO backchannel_migrate;

-- Preview migration role
CREATE ROLE backchannel_preview_migrate WITH LOGIN PASSWORD 'CHANGE_ME_STRONG_PASSWORD_2';
GRANT USAGE ON SCHEMA backchannel_preview TO backchannel_preview_migrate;
GRANT CREATE ON SCHEMA backchannel_preview TO backchannel_preview_migrate;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel_preview 
    GRANT ALL ON TABLES TO backchannel_preview_migrate;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel_preview 
    GRANT ALL ON SEQUENCES TO backchannel_preview_migrate;
```

### 4. Create Runtime Roles

These roles have limited DML privileges for the application:

```sql
-- Production runtime role
CREATE ROLE backchannel_runtime WITH LOGIN PASSWORD 'CHANGE_ME_STRONG_PASSWORD_3';
GRANT USAGE ON SCHEMA backchannel TO backchannel_runtime;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel 
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO backchannel_runtime;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel 
    GRANT USAGE, SELECT ON SEQUENCES TO backchannel_runtime;

-- Preview runtime role
CREATE ROLE backchannel_preview_runtime WITH LOGIN PASSWORD 'CHANGE_ME_STRONG_PASSWORD_4';
GRANT USAGE ON SCHEMA backchannel_preview TO backchannel_preview_runtime;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel_preview 
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO backchannel_preview_runtime;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel_preview 
    GRANT USAGE, SELECT ON SEQUENCES TO backchannel_preview_runtime;
```

### 5. Grant Permissions on Existing Objects (After Migrations)

Migrations create tables owned by the migration role, so the `ALTER DEFAULT PRIVILEGES` statements
in steps 3-4 (run as `postgres`) do not apply to them. After running migrations, connect **as each
schema's migration role** and run the grants below, plus default privileges for future tables:

```sql
-- as backchannel_migrate
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO backchannel_runtime;
ALTER DEFAULT PRIVILEGES IN SCHEMA backchannel GRANT USAGE, SELECT ON SEQUENCES TO backchannel_runtime;
REVOKE ALL ON TABLE backchannel._sqlx_migrations FROM backchannel_runtime;
-- (same for backchannel_preview_migrate / backchannel_preview_runtime)
```

Grants on existing tables:

```sql
-- Production
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA backchannel TO backchannel_runtime;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA backchannel TO backchannel_runtime;

-- Preview
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA backchannel_preview TO backchannel_preview_runtime;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA backchannel_preview TO backchannel_preview_runtime;
```

### 6. Verify Isolation

Ensure roles cannot access other schemas:

```sql
-- Production runtime should NOT see preview schema
SET ROLE backchannel_runtime;
\dt backchannel_preview.*;
-- Should return: Did not find any relations

-- Preview runtime should NOT see production schema
SET ROLE backchannel_preview_runtime;
\dt backchannel.*;
-- Should return: Did not find any relations

-- Reset to postgres
RESET ROLE;
```

## Connection Strings

### Production

**Migration** (for running `cargo run --bin backchannel-migrate`):
```
postgresql://backchannel_migrate.arfxachrbugnvbneyboe:[PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require
```

**Runtime** (for Vercel function):
```
postgresql://backchannel_runtime.arfxachrbugnvbneyboe:[PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require
```

### Preview

**Migration**:
```
postgresql://backchannel_preview_migrate.arfxachrbugnvbneyboe:[PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require
```

**Runtime**:
```
postgresql://backchannel_preview_runtime.arfxachrbugnvbneyboe:[PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require
```

## Environment Variables

### Production

```bash
DATABASE_URL=postgresql://backchannel_runtime.arfxachrbugnvbneyboe:[PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require
DATABASE_SCHEMA=backchannel
ADMIN_TOKEN_SHA256=[PRODUCTION_HASH]
```

### Preview

```bash
DATABASE_URL=postgresql://backchannel_preview_runtime.arfxachrbugnvbneyboe:[PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require
DATABASE_SCHEMA=backchannel_preview
ADMIN_TOKEN_SHA256=[PREVIEW_HASH]
```

## Running Migrations

### Production

```bash
export DATABASE_URL="postgresql://backchannel_migrate.arfxachrbugnvbneyboe:[PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require"
export DATABASE_SCHEMA="backchannel"
cargo run --bin backchannel-migrate
```

### Preview

```bash
export DATABASE_URL="postgresql://backchannel_preview_migrate.arfxachrbugnvbneyboe:[PASSWORD]@aws-0-us-east-1.pooler.supabase.com:5432/postgres?sslmode=require"
export DATABASE_SCHEMA="backchannel_preview"
cargo run --bin backchannel-migrate
```

## Vercel Environment Variables

Set these in Vercel dashboard or via CLI:

### Production

```bash
vercel env add DATABASE_URL production
# Paste runtime connection string

vercel env add DATABASE_SCHEMA production
# Enter: backchannel

vercel env add ADMIN_TOKEN_SHA256 production
# Paste production admin token hash
```

### Preview

```bash
vercel env add DATABASE_URL preview
# Paste preview runtime connection string

vercel env add DATABASE_SCHEMA preview
# Enter: backchannel_preview

vercel env add ADMIN_TOKEN_SHA256 preview
# Paste preview admin token hash
```

## Vercel Region

The function is pinned to `iad1` (Virginia) to colocate with Supabase us-east-1.

This is configured in `vercel.json`:
```json
{
  "regions": ["iad1"]
}
```

## Schema Isolation Benefits

1. **Cost**: Single project, free tier
2. **Isolation**: Preview cannot affect production data
3. **Simplicity**: No project management overhead
4. **Security**: Role-based access control enforces boundaries

## Important Notes

- **Session Pooler Required**: Use port 5432 (session pooler), not 6543 (transaction pooler)
- **Transaction pooler does NOT support**:
  - Prepared statements (required by sqlx)
  - Advisory locks (required for message ordering)
- **Free Plan Limits**:
  - Projects pause after 1 week of inactivity (auto-resume on connection)
  - 500 MB database size total (shared across both schemas)
  - No automatic backups (manual pg_dump required)
- **Passwords**: Use strong, unique passwords for each role
- **Admin Tokens**: Use separate admin tokens for production and preview
