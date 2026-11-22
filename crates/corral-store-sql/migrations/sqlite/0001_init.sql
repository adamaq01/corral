create table users (
    id blob primary key not null,
    subject text not null unique,
    handle text not null unique,
    email text,
    role text not null,
    status text not null,
    created_at text not null,
    default_sandbox text references sandboxes on delete set null
) strict;

create table ssh_keys (
    id blob primary key not null,
    user_id blob not null references users on delete cascade,
    label text not null,
    public_key text not null unique,
    created_at text not null
) strict;

create index ssh_keys_user_id on ssh_keys (user_id);

create table sandboxes (
    id text primary key not null,
    owner blob not null references users on delete cascade,
    name text not null,
    image text not null,
    size text not null,
    created_at text not null,
    unique (owner, name)
) strict;
