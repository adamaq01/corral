create table users (
    id uuid primary key,
    subject text not null unique,
    handle text not null unique,
    email text,
    role text not null,
    enabled boolean not null,
    created_at timestamptz not null
);

create table ssh_keys (
    id uuid primary key,
    user_id uuid not null references users on delete cascade,
    label text not null,
    public_key text not null unique,
    fingerprint text not null,
    created_at timestamptz not null
);

create index ssh_keys_user_id on ssh_keys (user_id);

create table sandboxes (
    id text primary key,
    owner uuid not null references users on delete cascade,
    name text not null,
    image text not null,
    size text not null,
    is_default boolean not null,
    created_at timestamptz not null,
    unique (owner, name)
);

create table credentials (
    user_id uuid not null references users on delete cascade,
    name text collate "C" not null,
    created_at timestamptz not null,
    updated_at timestamptz not null,
    primary key (user_id, name)
);

create table credential_values (
    user_id uuid not null references users on delete cascade,
    name text collate "C" not null,
    blob bytea not null,
    updated_at timestamptz not null,
    primary key (user_id, name)
);
