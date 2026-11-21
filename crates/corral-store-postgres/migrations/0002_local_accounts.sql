create table local_accounts (
    username text collate "C" primary key,
    password_hash text not null,
    role text not null,
    created_at timestamptz not null
);
