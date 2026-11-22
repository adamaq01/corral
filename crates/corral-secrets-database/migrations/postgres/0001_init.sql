create table credentials (
    user_id uuid not null,
    name text collate "C" not null,
    sealed bytea not null,
    primary key (user_id, name)
);
