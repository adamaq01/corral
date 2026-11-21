create table users (
    id text primary key not null,
    subject text not null unique,
    handle text not null unique,
    email text,
    role text not null,
    enabled integer not null,
    created_at text not null
) strict;

create table ssh_keys (
    id text primary key not null,
    user_id text not null references users on delete cascade,
    label text not null,
    public_key text not null unique,
    fingerprint text not null,
    created_at text not null
) strict;

create index ssh_keys_user_id on ssh_keys (user_id);

create table sandboxes (
    id text primary key not null,
    owner text not null references users on delete cascade,
    name text not null,
    image text not null,
    size text not null,
    is_default integer not null,
    created_at text not null,
    unique (owner, name)
) strict;

create table credentials (
    user_id text not null references users on delete cascade,
    name text not null,
    created_at text not null,
    updated_at text not null,
    primary key (user_id, name)
) strict;

create table credential_values (
    user_id text not null references users on delete cascade,
    name text not null,
    blob blob not null,
    updated_at text not null,
    primary key (user_id, name)
) strict;
