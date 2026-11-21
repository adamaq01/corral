create table local_accounts (
    username text primary key not null,
    password_hash text not null,
    role text not null,
    created_at text not null
) strict;
